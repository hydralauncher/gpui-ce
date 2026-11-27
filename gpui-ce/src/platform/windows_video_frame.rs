use crate::{DevicePixels, Size};
use std::sync::{Arc, Mutex};

/// The YCbCr encoding of a [`WindowsVideoFrame`]: the matrix and the value range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoColorSpace {
    /// BT.709 (HD video), limited range (16-235).
    Bt709Limited,
    /// BT.601 (SD video), limited range (16-235).
    Bt601Limited,
    /// BT.601, full range (0-255, JPEG).
    Bt601Full,
}

/// A decoded NV12 video frame in a Direct3D 11 texture that another device made.
///
/// Hydra patch. The texture is created with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE` and
/// `D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX`, and `handle` is its NT handle. The renderer opens it
/// on its own device with `OpenSharedResource1`, samples both planes in the surface shader, and
/// holds the keyed mutex (key 0) around the draw; the producer holds the same key around its
/// writes. No pixels pass through the CPU.
///
/// `id` names the texture for the renderer's cache: it must be unique for the texture's
/// lifetime and never reused for another texture. `keepalive` keeps the texture and its handle
/// open while a scene still shows the frame.
#[derive(Clone)]
pub struct WindowsVideoFrame {
    handle: isize,
    id: u64,
    size: Size<DevicePixels>,
    color_space: VideoColorSpace,
    _keepalive: Arc<dyn std::any::Any + Send + Sync>,
}

impl WindowsVideoFrame {
    /// A frame in the shared texture `handle` (an NT `HANDLE`), `size` in pixels (even sides).
    pub fn new(
        handle: isize,
        id: u64,
        size: Size<DevicePixels>,
        color_space: VideoColorSpace,
        keepalive: Arc<dyn std::any::Any + Send + Sync>,
    ) -> Self {
        Self {
            handle,
            id,
            size,
            color_space,
            _keepalive: keepalive,
        }
    }

    /// The texture's NT handle.
    pub fn handle(&self) -> isize {
        self.handle
    }

    /// The texture's cache key.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The frame's size in device pixels.
    pub fn size(&self) -> Size<DevicePixels> {
        self.size
    }

    /// How the frame's YCbCr values map to RGB.
    pub fn color_space(&self) -> VideoColorSpace {
        self.color_space
    }
}

/// The frame a video shows now, read by the renderer when it draws.
///
/// Hydra patch. Paint a slot (`SurfaceSource::WindowsVideoSlot`) instead of
/// a frame, then [`WindowsVideoSlot::set`] each new frame and call
/// [`crate::Window::request_present`]: the window draws its last scene
/// again, which reads the new frame, with no render, layout or paint.
#[derive(Clone, Default)]
pub struct WindowsVideoSlot(Arc<Mutex<Option<WindowsVideoFrame>>>);

impl WindowsVideoSlot {
    /// An empty slot (nothing is drawn until a frame is set).
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows `frame` from the next draw or present.
    pub fn set(&self, frame: WindowsVideoFrame) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(frame);
    }

    /// The frame shown now.
    pub fn current(&self) -> Option<WindowsVideoFrame> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl std::fmt::Debug for WindowsVideoSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("WindowsVideoSlot")
            .field(&self.current())
            .finish()
    }
}

impl std::fmt::Debug for WindowsVideoFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsVideoFrame")
            .field("id", &self.id)
            .field("size", &self.size)
            .field("color_space", &self.color_space)
            .finish_non_exhaustive()
    }
}
