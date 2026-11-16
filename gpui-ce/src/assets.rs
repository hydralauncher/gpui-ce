use crate::{DevicePixels, Pixels, Result, SharedString, Size, size};
use gpui_util::ResultExt;
use smallvec::SmallVec;

use image::{Delay, EncodableLayout, Frame};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt,
    hash::Hash,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::SeqCst},
    },
};

/// A read-only source of files that the app can load by path, such as a directory or a bundle.
///
/// Prefer [`AssetRegistry`] when you control the entries: it can hold preloaded bytes as well as
/// on-demand loaders. An `AssetSource` is useful when the source already owns the path lookup.
/// It converts into an [`AssetRegistry`] with `From`, and
/// [`crate::Application::with_assets`] accepts that conversion. The converted registry loads
/// each file on demand.
pub trait AssetSource: 'static + Send + Sync {
    /// Returns the bytes of the file at `path`, or `None` if there is no such file.
    ///
    /// The paths are the ones that [`AssetSource::list`] returns.
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>>;

    /// Returns the paths of the assets under `path`. Pass `""` to list all of them.
    ///
    /// Each returned path is one that [`AssetSource::load`] accepts.
    fn list(&self, path: &str) -> Result<Vec<SharedString>>;

    /// Loads every asset now and yields each one as an [`AssetEntry::PreLoaded`].
    ///
    /// It lists everything with `list("")`, loads each path, and skips the paths that fail or are
    /// missing. Pass the result to [`AssetRegistry::extend`].
    fn iter_preloaded(&self) -> Box<dyn Iterator<Item = (SharedString, AssetEntry)> + '_> {
        let paths = self.list("").unwrap_or_default();
        let iter = paths.into_iter().filter_map(move |path| {
            let entry = AssetEntry::from(self.load(&path).ok().flatten()?);
            Some((path, entry))
        });
        Box::new(iter)
    }

    /// Yields one [`AssetEntry::OnDemand`] per path. Each entry calls [`AssetSource::load`] on this
    /// source when the asset is requested.
    ///
    /// Nothing is read until an asset is requested, and each request reads the file again. A failed
    /// load is logged and reported as `None`. Pass the result to [`AssetRegistry::extend`].
    fn iter_ondemand(self: Arc<Self>) -> Box<dyn Iterator<Item = (SharedString, AssetEntry)>> {
        use gpui_util::ResultExt;
        let paths = self.list("").unwrap_or_default();
        let iter = paths.into_iter().map(move |path| {
            // constructing OnDemand for loading assets from shared:Arc only when they are requested
            let entry = AssetEntry::from({
                let source = self.clone();
                let path = path.clone();
                move || source.load(path.as_str()).log_err().flatten()
            });
            (path, entry)
        });
        Box::new(iter)
    }
}

impl AssetSource for () {
    fn load(&self, _path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

/// The bytes of one asset. `Cow::Borrowed` for data compiled into the binary, and `Cow::Owned` for
/// data read at runtime.
pub type AssetData = Cow<'static, [u8]>;
/// A callback that produces an asset's bytes when the asset is requested, or `None` if it can't.
///
/// The cache may call it from a worker thread, so the closure and everything it captures must be
/// `Send` and `Sync`.
pub type FnAssetLoader = Arc<dyn Fn() -> Option<AssetData> + 'static + Send + Sync>;
/// One asset in the [`AssetRegistry`]: its bytes, or a loader that produces them on request.
///
/// `Cow<'static, [u8]>` converts into the `PreLoaded` variant, and a closure that returns
/// `Option<AssetData>` converts into `OnDemand`.
pub enum AssetEntry {
    /// The bytes are ready now: static data compiled into the binary, or a file that was already
    /// read. The registry returns them without copying.
    PreLoaded(AssetData),
    /// The bytes come from a loader that runs on every [`AssetRegistry::load`]. The registry does
    /// not cache the result, so the loader decides whether to read the disk again.
    OnDemand(FnAssetLoader),
}
impl From<AssetData> for AssetEntry {
    fn from(value: AssetData) -> Self {
        Self::PreLoaded(value)
    }
}
impl<F> From<F> for AssetEntry
where
    F: Fn() -> Option<AssetData> + 'static + Send + Sync,
{
    fn from(value: F) -> Self {
        Self::OnDemand(Arc::new(value))
    }
}

/// Returned when a path is already registered in an [`AssetRegistry`]. It holds the duplicated path.
#[derive(thiserror::Error, Debug)]
#[error("An asset with the pathkey {0:?} already exists")]
pub struct DuplicateAssetPath(SharedString);

/// The named assets that the app loads by path, such as images, SVGs and fonts.
///
/// Set the registry with [`crate::Application::with_assets`] and read it with
/// [`crate::App::assets`]. Paths are exact lookup keys such as `"icons/star.svg"`; the registry
/// does not resolve them against the filesystem.
/// Each path has one [`AssetEntry`] and can be registered only once. Build a registry with
/// [`AssetRegistry::insert`], [`AssetRegistry::extend`] or `collect()`, or convert an
/// [`AssetSource`].
#[derive(Default)]
pub struct AssetRegistry {
    entries: BTreeMap<SharedString, AssetEntry>,
}

impl<T: AssetSource + 'static> From<T> for AssetRegistry {
    fn from(value: T) -> Self {
        let shared: Arc<dyn AssetSource> = Arc::new(value);
        Self::from(shared)
    }
}

/// Converts the provided [`AssetSource`] into a fresh [`AssetRegistry`] where all entries in
/// the asset source are loaded [OnDemand](AssetEntry::OnDemand) via [`AssetSource::iter_ondemand`].
impl From<Arc<dyn AssetSource>> for AssetRegistry {
    fn from(value: Arc<dyn AssetSource>) -> Self {
        let mut assets = Self::default();
        let _ = assets.extend(value.iter_ondemand());
        assets
    }
}

impl<KeyType, ValueType> FromIterator<(KeyType, ValueType)> for AssetRegistry
where
    KeyType: AsRef<str>,
    ValueType: Into<AssetEntry>,
{
    fn from_iter<T: IntoIterator<Item = (KeyType, ValueType)>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Self::default(), |mut assets, (key, value)| {
                assets.insert(key.as_ref(), value).log_err();
                assets
            })
    }
}

impl AssetRegistry {
    /// Adds the asset at `path`.
    ///
    /// The asset can be `Cow::Borrowed(&'static [u8])` for static bytes, `Cow::Owned(Vec<u8>)` for
    /// bytes you already have, or a closure that returns them on demand (see [`FnAssetLoader`]).
    ///
    /// # Errors
    ///
    /// Returns [`DuplicateAssetPath`] if `path` is already registered. The existing entry is kept.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// # use std::borrow::Cow;
    /// let mut assets = AssetRegistry::default();
    /// assets
    ///     .insert(
    ///         "icons/star.svg",
    ///         Cow::Borrowed(
    ///             &b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\"><rect width=\"8\" height=\"8\"/></svg>"[..],
    ///         ),
    ///     )
    ///     .unwrap();
    ///
    /// assert!(assets.load("icons/star.svg").is_some());
    /// assert!(assets.load("icons/missing.svg").is_none());
    /// assert!(assets.insert("icons/star.svg", Cow::Borrowed(&b""[..])).is_err());
    /// ```
    pub fn insert(
        &mut self,
        path: impl Into<SharedString>,
        asset: impl Into<AssetEntry>,
    ) -> Result<(), DuplicateAssetPath> {
        let path = path.into();
        if self.entries.contains_key(&path) {
            return Err(DuplicateAssetPath(path));
        }
        self.entries.insert(path, asset.into());
        Ok(())
    }

    /// Returns the bytes at `path`, or `None` if the path isn't registered.
    ///
    /// Preloaded entries are returned as [`Cow::Borrowed`] with no copy. An on-demand entry runs its
    /// loader on each call, so the bytes may be read from disk again. An on-demand loader that
    /// returns `None` makes this return `None` too.
    pub fn load<'this>(&'this self, path: impl AsRef<str>) -> Option<Cow<'this, [u8]>> {
        match self.entries.get(path.as_ref())? {
            AssetEntry::PreLoaded(Cow::Borrowed(data)) => Some(Cow::Borrowed(data)),
            AssetEntry::PreLoaded(Cow::Owned(data)) => Some(Cow::Borrowed(data.as_bytes())),
            AssetEntry::OnDemand(loader) => loader(),
        }
    }

    /// Adds each `(path, entry)` pair in order.
    ///
    /// # Errors
    ///
    /// Returns the [`DuplicateAssetPath`] of every pair whose path was already registered, in
    /// order. The other pairs are still added, and each duplicate is logged.
    pub fn extend<IterType, KeyType, EntryType>(
        &mut self,
        into_iter: IterType,
    ) -> Result<(), Vec<DuplicateAssetPath>>
    where
        IterType: IntoIterator<Item = (KeyType, EntryType)>,
        KeyType: AsRef<str>,
        EntryType: Into<AssetEntry>,
    {
        let mut result = Ok(());
        for (path_key, into_entry) in into_iter.into_iter() {
            if let Err(err) = self.insert(path_key.as_ref(), into_entry.into()) {
                gpui_util::log_err(&err);
                result = Err({
                    let mut errs: Vec<DuplicateAssetPath> = result.err().unwrap_or_default();
                    errs.push(err);
                    errs
                });
            }
        }
        result
    }

    /// Yields every file in a [`RustEmbed`](rust_embed::RustEmbed) folder as a preloaded entry.
    ///
    /// `T` is a type that derives `RustEmbed` for a folder. Each path is the file name relative to
    /// that folder. Pass the iterator to [`AssetRegistry::extend`]. This needs the
    /// `embedded-assets` feature of gpui, which also brings in the `rust-embed` crate.
    #[cfg(feature = "embedded-assets")]
    pub fn iter_embed<T: rust_embed::RustEmbed>()
    -> impl Iterator<Item = (Cow<'static, str>, AssetEntry)> + 'static {
        T::iter().filter_map(|path| {
            let file = T::get(path.as_ref())?;
            Some((path, AssetEntry::PreLoaded(file.data)))
        })
    }
}

/// Identifies a [`RenderImage`]. [`RenderImage::new`] gives each image a new ID, so two images
/// never share one. The wrapped `usize` is the counter value that identifies the image.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ImageId(pub usize);

#[derive(PartialEq, Eq, Hash, Clone)]
#[expect(missing_docs)]
pub struct RenderImageParams {
    pub image_id: ImageId,
    pub frame_index: usize,
}

/// A decoded image: one or more frames of BGRA pixels, ready to draw.
///
/// Create one with [`RenderImage::new`]. Two images are equal when their [`ImageId`]s are equal.
/// Still images have one frame. Animated images have one frame per step of the animation.
pub struct RenderImage {
    /// The ID that identifies this image. See [`ImageId`].
    pub id: ImageId,
    /// The scale factor of this image on render.
    pub(crate) scale_factor: f32,
    data: SmallVec<[Frame; 1]>,
}

impl PartialEq for RenderImage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RenderImage {}

impl RenderImage {
    /// Creates an image from decoded frames, with a new [`ImageId`].
    ///
    /// The frames are moved in, not copied. Pass a `SmallVec<[Frame; 1]>`, or anything that converts
    /// into one, such as a `Vec<Frame>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// use image::{Frame, ImageBuffer, Rgba};
    ///
    /// let pixels = ImageBuffer::from_pixel(2, 3, Rgba([255u8, 0, 0, 255]));
    /// let image = RenderImage::new(vec![Frame::new(pixels)]);
    ///
    /// assert_eq!(image.frame_count(), 1);
    /// assert_eq!(image.as_bytes(0).map(|bytes| bytes.len()), Some(2 * 3 * 4));
    /// ```
    pub fn new(data: impl Into<SmallVec<[Frame; 1]>>) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

        let data = data.into();
        image_memory::created(&data);
        Self {
            id: ImageId(NEXT_ID.fetch_add(1, SeqCst)),
            scale_factor: 1.0,
            data,
        }
    }

    /// Returns the pixels of frame `frame_index` as BGRA bytes, four per pixel, with rows packed and
    /// no padding.
    ///
    /// Returns `None` if there is no such frame.
    pub fn as_bytes(&self, frame_index: usize) -> Option<&[u8]> {
        self.data
            .get(frame_index)
            .map(|frame| frame.buffer().as_raw().as_slice())
    }

    /// Returns the size of frame `frame_index` in device pixels. Returns zero for a missing frame.
    pub fn size(&self, frame_index: usize) -> Size<DevicePixels> {
        self.data
            .get(frame_index)
            .map(|frame| {
                let (width, height) = frame.buffer().dimensions();
                size(width.into(), height.into())
            })
            .unwrap_or_default()
    }

    /// Get the size of this image, in pixels for display, adjusted for the scale factor.
    pub(crate) fn render_size(&self, frame_index: usize) -> Size<Pixels> {
        self.size(frame_index)
            .map(|v| (v.0 as f32 / self.scale_factor).into())
    }

    /// Returns how long to show frame `frame_index` before the next one. Returns 100 ms for a
    /// missing frame.
    pub fn delay(&self, frame_index: usize) -> Delay {
        self.data
            .get(frame_index)
            .map(|frame| frame.delay())
            .unwrap_or(Delay::from_numer_denom_ms(100, 1))
    }

    /// Returns the number of frames. A still image has one. A GIF or animated WebP has one per frame.
    pub fn frame_count(&self) -> usize {
        self.data.len()
    }
}

/// Hydra patch: an image nobody holds any more leaves the windows' atlases
/// (on each window's next draw) instead of staying there until the app calls
/// `Window::drop_image`, which most call sites never did.
impl Drop for RenderImage {
    fn drop(&mut self) {
        image_memory::dropped(self.id, &self.data);
    }
}

/// What the process holds for images and text (`HYDRA_MEM_STATS` in the
/// app): decoded images alive (their CPU frames) and the tiles the windows'
/// atlases hold (bytes of the tiles, not of the textures around them).
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryStats {
    /// `RenderImage`s alive.
    pub images: usize,
    /// Their frames' bytes.
    pub image_bytes: usize,
    /// Image frames uploaded to an atlas and not removed.
    pub atlas_image_tiles: usize,
    /// Their bytes.
    pub atlas_image_bytes: usize,
    /// Glyph rasters uploaded (glyphs are never removed).
    pub atlas_glyph_bytes: usize,
    /// SVG masks uploaded (never removed).
    pub atlas_svg_bytes: usize,
}

/// The current [`MemoryStats`].
pub fn memory_stats() -> MemoryStats {
    use std::sync::atomic::Ordering::Relaxed;
    let (atlas_image_tiles, atlas_image_bytes) = atlas_memory::images();
    MemoryStats {
        images: image_memory::LIVE.load(Relaxed),
        image_bytes: image_memory::LIVE_BYTES.load(Relaxed),
        atlas_image_tiles,
        atlas_image_bytes,
        atlas_glyph_bytes: atlas_memory::GLYPH_BYTES.load(Relaxed),
        atlas_svg_bytes: atlas_memory::SVG_BYTES.load(Relaxed),
    }
}

/// Hydra patch: images alive, and the images dropped that each window still
/// has to remove from its atlas (a cursor per window into one queue).
pub(crate) mod image_memory {
    use super::{Frame, ImageId};
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    pub(crate) static LIVE: AtomicUsize = AtomicUsize::new(0);
    pub(crate) static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

    struct Dropped {
        /// The sequence number of `entries[0]`.
        first: u64,
        entries: VecDeque<(ImageId, usize)>,
        /// Each window's token and the next entry it hasn't seen.
        cursors: Vec<(u64, u64)>,
        next_token: u64,
    }

    static DROPPED: Mutex<Dropped> = Mutex::new(Dropped {
        first: 0,
        entries: VecDeque::new(),
        cursors: Vec::new(),
        next_token: 0,
    });

    fn lock() -> std::sync::MutexGuard<'static, Dropped> {
        DROPPED.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bytes(frames: &[Frame]) -> usize {
        frames.iter().map(|f| f.buffer().as_raw().len()).sum()
    }

    pub(crate) fn created(frames: &[Frame]) {
        LIVE.fetch_add(1, Relaxed);
        LIVE_BYTES.fetch_add(bytes(frames), Relaxed);
    }

    pub(crate) fn dropped(id: ImageId, frames: &[Frame]) {
        LIVE.fetch_sub(1, Relaxed);
        LIVE_BYTES.fetch_sub(bytes(frames), Relaxed);
        let mut d = lock();
        if !d.cursors.is_empty() {
            d.entries.push_back((id, frames.len()));
        }
    }

    impl Dropped {
        /// Forget what every window has seen.
        fn trim(&mut self) {
            let Some(min) = self.cursors.iter().map(|c| c.1).min() else {
                self.first += self.entries.len() as u64;
                self.entries.clear();
                return;
            };
            while self.first < min && self.entries.pop_front().is_some() {
                self.first += 1;
            }
        }
    }

    /// A window's place in the queue of dropped images.
    pub(crate) struct DroppedImages {
        token: u64,
    }

    impl DroppedImages {
        pub(crate) fn new() -> Self {
            let mut d = lock();
            let token = d.next_token;
            d.next_token += 1;
            let end = d.first + d.entries.len() as u64;
            d.cursors.push((token, end));
            DroppedImages { token }
        }

        /// The images dropped since the last call (their ids and frames).
        pub(crate) fn take(&self) -> Vec<(ImageId, usize)> {
            let mut d = lock();
            if d.entries.is_empty() {
                return Vec::new();
            }
            let end = d.first + d.entries.len() as u64;
            let first = d.first;
            let Some(cursor) = d.cursors.iter_mut().find(|c| c.0 == self.token) else {
                return Vec::new();
            };
            let start = cursor.1.max(first);
            cursor.1 = end;
            let taken = d
                .entries
                .iter()
                .skip((start - first) as usize)
                .copied()
                .collect();
            d.trim();
            taken
        }
    }

    impl Drop for DroppedImages {
        fn drop(&mut self) {
            let mut d = lock();
            d.cursors.retain(|c| c.0 != self.token);
            d.trim();
        }
    }
}

/// Hydra patch: the atlas tiles uploaded, for [`memory_stats`].
pub(crate) mod atlas_memory {
    use super::ImageId;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    pub(crate) static GLYPH_BYTES: AtomicUsize = AtomicUsize::new(0);
    pub(crate) static SVG_BYTES: AtomicUsize = AtomicUsize::new(0);

    /// (atlas, image, frame) -> bytes.
    static IMAGES: Mutex<Option<HashMap<(usize, ImageId, usize), usize>>> = Mutex::new(None);

    pub(crate) fn uploaded_image(atlas: usize, id: ImageId, frame: usize, bytes: usize) {
        let mut images = IMAGES.lock().unwrap_or_else(|e| e.into_inner());
        images
            .get_or_insert_with(HashMap::default)
            .insert((atlas, id, frame), bytes);
    }

    /// Whether the tile was there.
    pub(crate) fn removed_image(atlas: usize, id: ImageId, frame: usize) -> bool {
        let mut images = IMAGES.lock().unwrap_or_else(|e| e.into_inner());
        images
            .as_mut()
            .is_some_and(|m| m.remove(&(atlas, id, frame)).is_some())
    }

    pub(crate) fn images() -> (usize, usize) {
        let images = IMAGES.lock().unwrap_or_else(|e| e.into_inner());
        images
            .as_ref()
            .map_or((0, 0), |m| (m.len(), m.values().sum()))
    }

    pub(crate) fn glyph(bytes: usize) {
        GLYPH_BYTES.fetch_add(bytes, Relaxed);
    }

    pub(crate) fn svg(bytes: usize) {
        SVG_BYTES.fetch_add(bytes, Relaxed);
    }
}

impl fmt::Debug for RenderImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("id", &self.id)
            .field("size", &self.data.first().map(|f| f.buffer().dimensions()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;

    #[test]
    fn empty_render_image_does_not_panic() {
        let image = RenderImage::new(SmallVec::new());
        assert_eq!(image.frame_count(), 0);
        assert_eq!(image.size(0), Size::default());
        assert_eq!(image.as_bytes(0), None);
        assert_eq!(image.render_size(0), Size::default());
        assert_eq!(image.delay(0), Delay::from_numer_denom_ms(100, 1));
        let _ = format!("{image:?}");
    }

    fn example_assets() -> Vec<(&'static str, AssetData)> {
        vec![
            ("asset0", Cow::Owned(vec![0u8])),
            ("asset1", Cow::Owned(vec![1u8])),
            ("asset2", Cow::Owned(vec![2u8])),
            ("asset3", Cow::Owned(vec![3u8])),
        ]
    }

    fn test_load_from(registry: &AssetRegistry) {
        for (key, value) in example_assets() {
            let loaded = registry.load(key).map(|cow| cow.to_vec());
            assert_eq!(loaded, Some(value.to_vec()));
        }
    }

    struct StubAssetSource;
    impl AssetSource for StubAssetSource {
        fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
            for (key, data) in example_assets() {
                if key == path {
                    return Ok(Some(data));
                }
            }
            unimplemented!()
        }

        fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
            Ok(example_assets()
                .into_iter()
                .map(|(key, _)| key.into())
                .collect())
        }
    }

    #[test]
    fn test_from_assetsource() {
        let assets = AssetRegistry::from(StubAssetSource);
        test_load_from(&assets);
    }

    #[test]
    fn test_from_arc_assetsource() {
        let arc: Arc<dyn AssetSource> = Arc::new(StubAssetSource);
        let assets = AssetRegistry::from(arc);
        test_load_from(&assets);
    }

    #[test]
    fn test_fromiter_assetdata() {
        let assets = AssetRegistry::from_iter(example_assets());
        test_load_from(&assets);
    }

    #[test]
    fn test_fromiter_assetentry() {
        let assets = AssetRegistry::from_iter(
            example_assets()
                .into_iter()
                .map(|(key, value)| (key, AssetEntry::from(value)))
                .chain(std::iter::once((
                    "asset6",
                    AssetEntry::OnDemand(Arc::new(|| Some(AssetData::Owned(vec![6u8])))),
                ))),
        );
        test_load_from(&assets);
        assert_eq!(
            assets.load("asset6").map(|cow| cow.to_vec()),
            Some(vec![6u8])
        );
    }

    #[test]
    fn test_insert() {
        let mut assets = AssetRegistry::default();
        for (key, value) in example_assets() {
            let result = assets.insert(key, value);
            assert!(result.is_ok());
        }
        test_load_from(&assets);
    }

    #[test]
    fn test_extend() {
        let mut assets = AssetRegistry::default();
        let result = assets.extend(
            example_assets()
                .into_iter()
                .map(|(key, value)| (key, AssetEntry::PreLoaded(value))),
        );
        assert!(result.is_ok());
        test_load_from(&assets);
    }

    #[test]
    fn test_extend_assetsource_preloaded() {
        let mut assets = AssetRegistry::default();
        let result = assets.extend(StubAssetSource.iter_preloaded());
        assert!(result.is_ok());
        test_load_from(&assets);
    }

    #[test]
    fn test_extend_assetsource_ondemand() {
        let mut assets = AssetRegistry::default();
        let result = assets.extend(Arc::new(StubAssetSource).iter_ondemand());
        assert!(result.is_ok());
        test_load_from(&assets);
    }
}
