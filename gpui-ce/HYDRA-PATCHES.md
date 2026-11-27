# gpui-ce 0.3.0, patched for Hydra

This is the crates.io release of gpui-ce 0.3.0 (src, build.rs, docs and
examples, which the crate includes at compile time). The example, test and
bench targets are removed from Cargo.toml. The workspace uses it through
`[patch.crates-io]`.

Patches:

- `src/lerp.rs`: `Hsla` interpolation is premultiplied by alpha, as CSS
  transitions are. Without it, a transition between a transparent color and
  an opaque one (for example `white(0.0)` to `#262626`) passes through a
  lighter color. QA saw this as flicker on segmented controls, hovers and
  buttons.
- `src/svg_renderer.rs`: every SVG alpha mask has a 2-texel transparent
  border. `paint_svg` centers the tile, so icons don't move. A rotated icon
  (the trailer spinner, section chevrons) sampled past its tile and showed
  stray pixels from the neighbouring atlas tile.
- `src/window.rs` + `src/scene.rs`: element opacity multiplies into each
  primitive's alpha (`Scene::push_opacity`) instead of an offscreen group.
  The group switched subpixel text to grayscale while an element faded or
  was pressed (QA saw buttons flicker) and drew a full-window pass for every
  faded element. Overlapping children inside a faded element now blend with
  each other, as in GPUI before content-filter groups. An element that needs
  the browser's group (overlapping children) opts in with `opacity_group()`
  ("Opacity groups, opt-in" below).
- `src/window.rs` (`Window::moved`): a window move redraws only when the
  scale factor or content size changed. GPUI refreshed every view (cache
  bypassed) on each move event, so dragging the window lagged.

- `src/window.rs` (`frame_stats`): `HYDRA_FRAME_STATS=1` prints each two
  seconds of drawn frames' draw (render, layout, prepaint, paint) and
  present times on the UI thread (mean, p50, p95, max), for the
  measurements in `docs/performance.md`. Off, it costs one cached env check
  per frame.
  It also lists what asked for those frames: the call sites of
  `request_animation_frame`, `on_next_frame`, `refresh` and `notify`
  (`#[track_caller]` on them and on `App`'s `notify`), with how many times
  per frame, so a window that never settles names its cause.

### Element transforms and edge masks (2026-11-11)

CSS `transform` / `transform-origin` and `mask-image: linear-gradient(..)`
for an element and everything it paints, so animations no longer have to be
faked (growing boxes, scaled fonts, per-item fades, fading into the page
color). The app API is `Styled::transform(ElementTransform)` and
`Styled::mask(EdgeMask)` (`src/element_transform.rs`); `docs/agent-guide.md`
in the app has the how-to. The shaders live in `../gpui-ce-render` (below),
which every renderer compiles from, so DirectX (Windows), Vulkan/wgpu
(Linux) and Metal (macOS) all get the same code.

- `src/element_transform.rs` (new): `ElementTransform` (a 2D affine matrix
  built like a CSS transform list, plus its origin, default the center) and
  `EdgeMask` (a fade per edge: transparent up to `start`, opaque from `end`,
  measured inward; fractions of the element's size). Both are `Style`
  fields (`src/style.rs`, `Style::transform_and_mask`) with `Styled`
  builders (`src/styled.rs`).
- `src/elements/div.rs` (`Interactivity::prepaint` / `paint`): the
  transform, then the mask, wrap the element's hitbox, box (shadows,
  background, border) and children, in both phases. That covers `div`,
  `img`, `svg` and `uniform_list`; `src/elements/canvas.rs`, `list.rs`,
  `surface.rs`, `image_cache.rs` and `container_query.rs` do the same for
  theirs (2026-11-11, second batch: those four ignored their own).
- `src/window.rs` (`Window::defer_draw`): a deferred draw requested inside
  a transform gets its clip in window space (it paints later, untransformed,
  like a portal). `Window::defer_draw_transformed` (`deferred(..)
  .transformed(true)`, `src/elements/deferred.rs`) keeps the total
  transform with the draw instead and prepaints and paints it inside that
  transform again (`with_total_element_transform`), so a popover inside a
  transformed element turns and scales with it, hitboxes included, as a CSS
  `z-index` popover does.
- `src/window.rs`: `Window::with_element_transform` keeps a stack of total
  transforms (layout space to window, logical pixels). Inside it the
  content mask is re-expressed in the element's own coordinates (the
  enclosing clip comes in as its bounding box: exact for scales and
  translations, a box around it for rotations), so clips and fades set
  inside the element turn and scale with it. Hitboxes store the inverse
  transform and `Hitbox::contains` tests in element space: hover, clicks
  and scrolling follow the transform. `Window::element_transform` and
  `Window::point_in_element_space` expose it. Rotated text is rasterized in
  grayscale (subpixel coverage only fits an unrotated grid; scaled text
  keeps subpixel AA so it doesn't switch at the end of an animation).
- `src/scene.rs`: one transform per primitive. `Quad`, `Shadow`,
  `Underline`, `MonochromeSprite`, `SubpixelSprite` and `PolychromeSprite`
  carry `element_transform` (a `TransformationMatrix`, 24 bytes) that the
  vertex shader applies; `Scene::insert_primitive` stamps the current one.
  Paths are transformed vertex by vertex. Surfaces (video), backdrop
  filters and filter groups (blur) are drawn from window-space rectangles:
  they get the transformed bounding box (exact for scales, not rotated) and
  their blur radii scale with the transform, as CSS filters work in the
  element's space. Draw order (the bounds tree) and layers use window-space
  boxes. Replayed (cached) output is inserted as recorded, without applying
  the current opacity or transform again (the old layer path re-applied the
  opacity, a pre-existing double fade). `TransformationMatrix` gained
  `inverse`, `is_unit`, `is_translation`, `rotates`, `scale_magnitude`,
  `to_scaled`, `apply_scaled`, `apply_to_mask`, `apply_to_logical_mask` and
  `apply_to_bounds`.
- `src/view.rs`: a cached view's output is keyed on the element transform
  too (its primitives and hitboxes carry it), so a view inside an animated
  transform draws again each frame instead of replaying stale output.
- `src/window.rs` (`ContentMask`): `fade_start` per edge (where the fade
  begins, inward from the edge) next to `fade_out` (its length), which is
  what an `EdgeMask` becomes. `ContentMask::intersect` now carries a
  parent's fade into a child that clips closer (with a negative start)
  instead of dropping it, so a masked list keeps its fade when its rows clip
  themselves; when both masks fade one edge, the one opaque further in
  wins. `paint_quad`'s border strips keep the mask's fades (they dropped
  them). `ContentMask` is 48 bytes now (`src/scene/abi.rs`).
- `src/scene.rs` (`Scene::push_layer`) and `src/style.rs`
  (`ScaledFilter::scale_by`): window-space layer bounds, filter scaling.

Limits, by design or left for later:

- Layout ignores transforms (as CSS does); code that compares element
  bounds with the mouse position by hand (not through a hitbox) doesn't
  follow them. Use `Window::point_in_element_space`.
- Plain `deferred` (and the app's window layers, which stand for React
  portals) draws untransformed at window level, as a portal escapes its
  ancestors' transforms in CSS; `.transformed(true)` opts a deferred
  element into its ancestors' transforms.
- A rotated element's clips are tested along its own axes (inner clips) or
  as a box (the enclosing clip: exact for scales, translations and quarter
  turns; a box around it for other angles), and a rotated video or
  backdrop/filter group gets its bounding box, not a rotated rectangle. An
  exact rotated outer clip needs a second, window-space clip per primitive
  in every shader and backend (16 more bytes per primitive); no screen needs
  it yet.
- Edge antialiasing: scaled or rotated quads, images and sprites are drawn
  a device pixel past their bounds and fade across their edges; clip edges
  under a rotation are hard. Distances in a scaled element are measured in
  its own units, so a 0.5 scale antialiases over half a pixel.
- Text keeps its glyph rasters and is sampled through the transform (like a
  composited layer in Chromium); it isn't re-rasterized at the new size.
- A zero scale draws at 1e-4 (invisible) so children still prepaint.

### Chromium's focus ring (2026-11-11)

- `src/style.rs`: `Style::focus_ring: Option<Hsla>` paints Chromium's
  `outline: auto` (`outline_painter.cc` `PaintFocusRing`, light color
  scheme) over the element and its children: a 2px band in the color
  centered on the border box, outset by -1px when every border is at least
  1px wide (the ring takes the border) and 0 otherwise, a 1px white ring
  just outside it, the corners following the border radius (at least 3px).
  The app sets it in `focus_visible` (`components/focus.rs`).
- `src/style.rs`: `Style::outline: Option<Outline>` (2026-11-12) paints CSS
  `outline: <width> solid <color>; outline-offset: <offset>` over the
  element: a ring `offset` outside the border box (inside when negative),
  `width` wide, its corners the border radius plus the offset (square on a
  square box), for the `:focus-visible` outlines Electron's stylesheets set
  in place of the focus ring.
- `src/window.rs`: `Window::is_focus_rendered(&FocusHandle)` (2026-11-12):
  whether the element tracking a handle was drawn in the last frame, so a
  closing layer gives the focus back only to an element that is still there
  (a handle outlives its element: a closed menu's, a page left behind).
- `src/tab_stop.rs`, `src/elements/div.rs` (2026-11-12): tab order anchors.
  GPUI orders tab stops by paint order, the DOM by document order; an
  element painted over content the document puts after it (a sticky header
  over its rows, a lightbox's buttons over the slides, a button over a
  banner) came last. `InteractiveElement::tab_anchor(&TabAnchor)` records
  the place (the tab map's length after the element's own stop, before its
  children's) when the element paints; `tab_before(&TabAnchor)` on an
  element painted later puts its stop there. A node sorts on (path,
  `order`, insertion index), `order` being twice the insertion index, or
  one less than twice the anchor for an anchored stop; the history records
  an anchored stop as `InsertBefore(handle, distance back to the anchor)`,
  so a cached view's replay rebuilds the same order. Unit test
  `tab_stop::tests::test_insert_before` (cargo can't run it here: the
  patched crate isn't a workspace member and has dev-dependencies; the
  order was checked in the app under Xvfb).

### CSS behaviour QA ran into (2026-11-11, third batch)

- `src/style_transitions.rs`: an unset optional property transitions from
  and to its CSS initial value (`TransitionInitial`, a type parameter of the
  property's state): `opacity` 1, a background, border or text background
  transparent; properties that inherit or are `auto` (text color, font
  size, line height, letter spacing, line clamp, aspect ratio) don't animate
  to or from unset. GPUI used `T::default()`: opacity 0, so a button enabling
  from 0.5 faded towards 0 and popped to 1, and palette's `Hsla::default()`,
  opaque black, so a row with no background went black on hover before
  reaching its color.
- `src/color.rs`: `Background::default()` is transparent, as its doc says (it
  was palette's opaque-black `Hsla::default()`).
- `src/style.rs` (`paint_box`): the border paints before the children, as
  CSS paints a box (background, border, then the content). GPUI drew it over
  the children, so art hanging over a card's edge showed the border across
  it.

### Image memory (2026-11-12)

- `src/assets.rs`: `RenderImage` counts the images alive and their frames'
  bytes, and on drop queues its id for every window (`image_memory`, one
  cursor per window into one queue). `memory_stats()` (public) reports
  them, with the image tiles each window's atlas holds and the glyph and
  SVG bytes uploaded (`atlas_memory`); the app prints it with
  `HYDRA_MEM_STATS=1`.
- `src/window.rs`: at the start of each draw a window removes from its
  atlas the tiles of the images dropped since (`release_dropped_images`).
  GPUI only removed them on `Window::drop_image`, which most call sites
  never called, so an image nobody held stayed in the atlas for the life of
  the window. When a removed tile was still there, the cached views draw
  again that frame (one might replay a sprite of it). `drop_image` and the
  uploads in `paint_image` / `paint_svg` / `paint_glyph_from_atlas` keep the
  accounting.

### Animated images on screen (2026-11-12)

- `src/elements/img.rs` (`Img::paint`) + `src/window.rs`
  (`Window::painting_image_bounds`): while an `img` paints, the window holds
  its bounds, so an `ImageSource::Custom` source can tell whether its picture
  is inside the content mask. The app's animations (`anim_image.rs`, frames
  decoded while they play) stop advancing and decoding off screen, as a
  browser does. `None` while the image lays out and outside `img`.

### Opacity groups, opt-in (2026-11-12)

CSS composites an element with `opacity` as one picture: its children are
drawn, then the result fades. The per-primitive opacity above fades each
child on its own, so where children overlap (text and an icon over a card's
body, a cover over its shadow) the lower ones show through mid-fade. QA saw it
on the achievement notification.

- `src/style.rs` (`Style::opacity_group`), `src/styled.rs`
  (`Styled::opacity_group()`): the element's opacity is drawn as an
  offscreen group instead. Off by default, so buttons and every other faded
  element keep the per-primitive path and their subpixel text.
- `src/window.rs` (`Window::with_element_opacity_group`): while the opacity
  is between 0 and 1 (times any per-primitive opacity around it), the
  element and its children paint into a group composited once at that
  opacity (GPUI's content-filter group with no filter, which the DirectX,
  wgpu and Metal renderers already draw). At 1 or 0 there is no group (the
  children still paint, for their listeners).
- `src/scene.rs` (`Scene::push_opacity_group` / `pop_opacity_group`): the
  children paint opaque into the group; the scene tracks the window-space
  box of what they paint (a drop shadow's blur, overflow and nested filter
  groups included, replayed views too) and, when the group ends, gives both
  markers that box rounded out to device pixels, so the composite covers the
  element's painted area, not the window (GPUI's original group used the
  whole viewport for every faded element). The recorded start marker is
  updated with it, so a cached view replays the same box.
- Text: inside an open group, glyphs rasterize in grayscale (an offscreen
  target has no backdrop for subpixel coverage), as Chromium does in a
  layer whose opacity animates. Outside the group nothing changes; the
  group exists only while the opacity is below 1, so a settled element
  draws exactly as before. A frame with a group renders through the
  offscreen scene target (one extra blit), as a frame with a blur does.

Used by the achievement notification's card and chip, the leaderboard
toasts' entry and the download flyer (the app's `opacity_group()` calls).
`tools/render-check` checks it on DirectX (a group shows only the top square
where two overlap, per primitive the lower one shows through; the group's
box includes a shadow 30px below the element); `HYDRA_DEV_TRANSFORMS=1`
shows both side by side.

### Mask groups, opt-in (2026-11-15)

CSS `mask-image` masks an element's whole picture: its children are drawn,
then the result is faded. `Styled::mask` fades each primitive on its own, so
where children are stacked (a classics cover over its case, a card over a
backdrop) the lower ones show through inside the fade: on Big Picture's
profile library shelf Neo's alpha read 0.66 against Chromium's 0.485 at x
1095.

- `src/style.rs` (`Style::mask_group`), `src/styled.rs`
  (`Styled::mask_group()`): the element's mask is applied to its picture.
  Off by default.
- `src/elements/div.rs` (paint): a masked element with the flag paints
  through `Window::with_element_mask_group` instead of
  `with_content_mask`.
- `src/window.rs` (`Window::with_element_mask_group`): the children are
  clipped to the mask's box but not faded and paint into an offscreen
  group (the opacity group's pass, at opacity 1); the group's markers carry
  the mask in window space. Falls back to the per-primitive fade for a mask
  without a fade, under a scale or rotation (a translation is followed),
  and on macOS (until 2026-11-15: off Windows).
- `src/scene.rs` (`Scene::push_mask_group`; `pop_opacity_group` keeps a mask
  group's mask on its markers, where an opacity group gets its painted box).
- `../gpui-ce-render` (`BlurCompositeClip::EdgeFade`,
  `BlurUniforms::with_edge_fade`, `edge_fade_alpha` in the blur composite
  fragment): the composite is clipped to the mask's box less each edge's
  transparent start and fades in over each edge's length. The four lengths
  ride in `direction`, `standard_deviation` and `sample_step`, which a
  composite doesn't read, so `BlurUniforms` stays 112 bytes (crates.io's
  `gpui_ce_wgpu` asserts that size).
- `../gpui-ce-windows` (`dx_blur_and_composite`'s `edge_fade`): a content
  filter group whose content mask fades an edge composites through it.
  `../gpui-ce-wgpu` does the same since 2026-11-15; only the Metal renderer
  (crates.io) keeps the per-primitive fade.

Text inside the group is grayscale (an offscreen target), as in an opacity
group. Used by Big Picture's focus carousel (`mask-image:
linear-gradient(90deg, #000 calc(100% - 240px), transparent)` on Home's
carousels, the profile shelves and the game page's similar games).

### Transform groups, opt-in (2026-11-15)

Chromium draws a transformed element that is its own layer (a `filter`, a
3D rendering context, `will-change: transform`) by rasterizing it in its own
space and drawing that raster through the transform: its pixels are sampled
twice. GPUI draws each primitive through the transform, sampling once. On
Big Picture's classics cover case (a `drop-shadow` filter, turned 10deg)
the reference's front came out a step softer than any single sample.

- `src/style.rs` (`Style::transform_group`), `src/styled.rs`
  (`Styled::transform_group()`): off by default.
- `src/elements/div.rs` (paint): such an element paints through
  `Window::with_element_transform_group` instead of
  `with_element_transform`.
- `src/window.rs` (`Window::with_element_transform_group`): the element and
  its children paint with the unit transform (layout space) into an
  offscreen group (the opacity group's pass), composited through the total
  transform. Falls back to `with_element_transform` on macOS (off Windows
  until the wgpu renderer was vendored, 2026-11-15), outside paint and for a
  transform that collapses an axis. Inside the group
  `element_transform()` reads the unit (an element's own rectangle probe
  there gets its layout box) and text is grayscale.
- `src/scene.rs` (`Scene::push_transform_group`, `GroupTransform`,
  `FilterBoundary::transform`): the group tracks what it painted in layout
  space; when it ends, its markers get the screen box of that box through
  the transform (clipped to the window-space clip around the element) and
  `transform: Some(GroupTransform { matrix, source })`. Every other
  boundary has `None` (a new field: crates.io's renderers only build
  `FilterBoundary` in their tests).
- `../gpui-ce-render` (`BlurCompositeClip::Transform`,
  `BlurUniforms::with_transform`, `transform_composite_color` in the blur
  composite fragment): each output pixel samples the group's texture at the
  inverse transform (bilinear), inside the source box, its edges
  antialiased over an output pixel. The inverse rides in `corner_radii` and
  `source_size`, the source box in `direction`, `standard_deviation` and
  `sample_step`: `BlurUniforms` keeps its 112 bytes.
- `../gpui-ce-windows` (`dx_blur_and_composite`'s `group_transform`): an
  isolated group whose boundary carries a transform composites through it;
  `../gpui-ce-wgpu` (`composite_texture`'s `CompositeExtras`) too.
- The group is drawn as cc draws a render surface: its contents at the
  transform's scale components (`ComputeTransform2dScaleComponents`, the
  matrix's column lengths), the surface through the rest of the transform.

Measured on Big Picture's classics case (`HYDRA_BP_CASE_GROUP=1`, 1920x1080
`catalogue-classics`): 2.92% without the surface scale, 2.36% with it,
against 1.93% for the per-primitive drawing with the fitted CPU softening,
which the app keeps by default. Chromium's two samples there carry a
constant fractional offset (its fitted kernels are bilinear at 0.28 then
0.72 across, 0.11 then 0.89 down); this pass samples at the transform's own
phase. Not done: drawing the group at that sub-pixel offset.

Limits: the group renders into the window-sized offscreen target in layout
space, so content laid out outside the window is lost even when its
transformed picture would be on screen; one transform group can't also be a
mask group.

### Backdrop blurs fade out like CSS (2026-11-12)

QA (#hydra-neo) saw the page behind a closing modal stay soft until the
modal was gone, where Electron's backdrop clears as it fades.

- `src/scene.rs` (`soften_small_backdrop_blur`, from `insert_primitive`):
  every renderer blurs a backdrop at half resolution and scales it back up,
  so any radius above zero, even a tenth of a pixel, showed the downsampled
  picture. A backdrop filter animating to `blur(0px)` (the modal backdrop's
  0.4s `backdrop-fade-out`) kept the page soft for its whole run. Below
  `MIN_BACKDROP_BLUR` (2 device px) the filter now blurs at that radius and
  composites at `radius / 2` of its opacity over the sharp backdrop already
  in the frame, so it eases into the unblurred page. Larger radii are
  unchanged. Applied after the element transform scales the radius.
- `src/scene.rs` (`Primitive::fade`): a backdrop filter takes the element's
  opacity, as CSS composites the filtered backdrop inside the element's
  group (a menu with `backdrop-filter` fading in brought its blur in at full
  strength at once). The renderers already multiply `BackdropFilter::opacity`
  into the composite.

### Video frames on the GPU, Windows (2026-11-27)

Hydra's trailers decoded on the GPU (D3D11VA), then copied each frame back
to system memory, converted it to BGRA on the CPU and uploaded it to the
sprite atlas as a new image. Discord's GPUI client (Phoenix) keeps frames on
the GPU as NV12; this does the same.

- `src/platform/windows_video_frame.rs`: `WindowsVideoFrame`, an NV12
  texture another Direct3D 11 device shares through an NT handle and a keyed
  mutex (key 0 on both sides), with a cache id, its size, its
  `VideoColorSpace` (BT.709 or BT.601, limited or full range) and a
  keepalive the producer drops when the scene no longer shows it.
- `src/elements/surface.rs`: `SurfaceSource::WindowsVideo` and
  `SurfaceSource::size` made public.
- `src/scene.rs` + `src/window.rs`: `PaintSurface::corner_radii`, set by the
  new `Window::paint_rounded_surface` (clamped and scaled as
  `paint_image` does); `paint_surface` passes none. `PaintSurface::opacity`
  takes the element opacity (`Primitive::fade`), as a sprite does; surfaces
  ignored it before.
- `src/platform/windows_video_frame.rs` + `src/window.rs`:
  `WindowsVideoSlot`, the frame a video shows now
  (`SurfaceSource::WindowsVideoSlot`), read by the renderer when it draws,
  and `Window::request_present`, which draws the last scene again with no
  render, layout or paint (GPUI's existing `needs_present` path). A player
  paints its slot once; each new frame goes into the slot and only presents.
  Hydra's game page re-rendered (render, layout, paint of the whole page) for
  every trailer frame: the process went from 24% to 9% of a core.

## gpui_ce_render 0.3.0 (`../gpui-ce-render`)

The crates.io release (src, build.rs, Cargo.toml), used through
`[patch.crates-io]`. Its build script translates the Rust-authored shaders
in `src/shaders` (wgsl-rs) into WGSL (wgpu: Vulkan on Linux), HLSL compiled
to DXBC with D3DCompile (DirectX 11 on Windows; Windows builds only) and
MSL (Metal on macOS), and checks the GPU struct layouts against
`gpui::SCENE_BUFFER_LAYOUTS`. Same license (Apache-2.0).

Patches (element transforms and edge masks, see above):

- `src/shaders/common.rs`: `ContentMask` has `fade_start` and
  `ContentMask::alpha` ramps from it. New helpers: `element_vertex` /
  `sprite_element_vertex` (the rectangle corner in element space, through
  the sprite's own `transformation` for SVGs, then `element_transform` to
  clip space), `is_linear_identity`, `edge_antialias_margin` (grow a scaled
  or rotated rectangle by about a device pixel), `dilate_bounds`,
  `clamp_to_tile` and `tile_edge_coverage` / `sprite_texel_to_pixels`
  (sample a grown sprite inside its atlas tile and fade across the tile's
  edge, so rotated glyphs don't pick up their neighbours or show a hard
  cut).
- `src/shaders/quads.rs`, `shadows.rs`, `sprites.rs` (underlines,
  monochrome, subpixel and polychrome sprites): each struct has
  `element_transform`; each vertex shader maps through it and passes the
  element-space position as a new varying (`local_position`); the fragment
  shaders use it instead of the window position for SDFs, gradients, clip
  distances and fades. A transformed quad skips the fill-only fast path so
  its grown edge gets distance-based coverage. Paths, surfaces and blur
  passes are unchanged (the scene transforms them on the CPU).
- `src/shaders/host.rs`: `ContentMask` conversion with `fade_start`.

Checked with `tools/render-check` (DirectX, a hidden window rendered to a
PNG with pixel checks) and `HYDRA_DEV_TRANSFORMS=1` in the app (Linux under
Xvfb). macOS: Naga generates the MSL on every build (from the module that
validated), and the Metal renderer compiles it at startup; nobody has run
it on a Mac yet.

### Surfaces: NV12 color spaces and rounded corners (2026-11-27)

- `src/shaders/common.rs` + `filters.rs`: `SurfaceColorFormat` adds
  `Yuv709Limited` and `Yuv601Limited` (the existing `Yuv` stays BT.601 full
  range for CoreVideo). `SurfaceUniforms` replaces four padding words with
  `corner_radii` (16-byte aligned, the same size); the fragment shader
  multiplies in a rounded-rectangle coverage when a radius is set.
- `src/shaders/interface.rs`: the layout table follows.

### Sprites sample inside their atlas tile (2026-11-12)

`src/shaders/sprites.rs`: monochrome (SVG icons, grayscale glyphs) and
polychrome (images) sprites clamp their atlas position to half a texel inside
their tile (`clamp_to_tile`) always, not only under a scaling or rotating
element transform. An icon turned by its own `transformation` (a section
chevron rotating as it collapses, `svg().with_transformation`) samples between
texels right up to its quad's edge, and the bilinear filter mixed in the
neighbouring tile's texels there: QA kept seeing single white dots next to
the profile's and game page's collapsible widgets while their chevrons
turned. The 2-texel transparent border around SVG tiles (above) doesn't help
there, since the quad's edge is the tile's edge. A picture drawn larger than
its pixels bled the same way at its edges. An upright, unscaled sprite
samples texel centres, which the clamp leaves as they are. The edge fade for
scaled or rotated elements is unchanged.

### Clips inside element transforms are not snapped (2026-11-18)

`src/window.rs`: `Window::snapped_content_mask` grows the clip to whole
device pixels (`cover_bounds`) in the current element space. Inside an
element transform that space is the window shifted or scaled by a fraction of
a pixel, so the grown clip landed up to 1 device px past the real clip on
screen, a different px as the offset moved: a translated carousel track lit
the column just past its viewport, and a drifting hero art the row and column
just outside the hero, flickering. Inside a transform the clip is now the
exact mask (`with_element_transform` already maps it exactly); untransformed
content is snapped as before.

### Outer shadows stop at the element (2026-11-12)

`src/shaders/shadows.rs`: `shadow_coverage` and `smoothed_shadow_coverage`
multiply a drop shadow's coverage by one minus the element's own rounded-rect
coverage (`element_bounds`, `element_corner_radii`, which the primitive already
carried for inset shadows). CSS clips an outer `box-shadow` to outside the
border box; GPUI filled it under the element too, so a translucent element
with a shadow looked opaque (the profile's "Upload banner" button,
rgba(0,0,0,.3) over a 0.8 black shadow). Opaque elements look the same.

## gpui_ce_elements 0.3.0 (`../gpui-ce-elements`)

The crates.io release's `src`, with a Cargo.toml without the example and
test targets or dev-dependencies, used through `[patch.crates-io]` too.
Same license as gpui-ce (Apache-2.0, `LICENSE-APACHE` here).

Patches (the review and comment editor, `hydra-gpui/src/components/rich_editor.rs`,
edits rich text in one field the way Electron's TipTap editor does):

- `src/editable_text/element.rs` (the measure callback): the shaped-layout
  cache compares against the layout the state holds now, not the snapshot
  taken when the element was built. Taffy measures a text area more than
  once per frame (min-content, then its width) and each measure stores its
  layout; with the snapshot, every other frame the width measure hit the
  cache and returned without storing, so the min-content layout was painted.
  With break-word those two layouts differ for a long word, and a wrapped
  chat input flickered between one line and several (2026-11-23).

- `src/editable_text/element.rs`: `EditableTextElement::text_highlights`
  lays `HighlightStyle`s over byte ranges of the text (bold, italic,
  underline, strikethrough, code). The field shaped its text as one run;
  it now builds one run per range, as `StyledText` does, and the ranges are
  part of the layout cache key (`layout.rs`). Without highlights the field
  shapes exactly as before.
- `src/editable_text/state.rs`: `EditableTextState::selection_bytes`
  (anchor and caret as byte offsets; the public API only gave UTF-16 ranges
  through the IME handler) and `replace_bytes` (a ranged, undoable edit that
  emits `TextChanged`; only `emplace`, a whole-text replace, was public).

## gpui_ce_windows 0.3.0 (`../gpui-ce-windows`)

The crates.io release (src, Cargo.toml, LICENSE), used through
`[patch.crates-io]` (2026-11-11). Same license (Apache-2.0).

- `src/events.rs` (`handle_cursor_changed`): a new cursor style applies at
  once when the pointer is over the window (a WM_SETCURSOR sent with the real
  hit test, so the resize borders keep theirs). GPUI only called SetCursor
  when the cursor was hidden or shown and otherwise waited for the next
  WM_SETCURSOR, a mouse move: the not-allowed cursor stayed over a button
  that had just become enabled until the mouse moved.
- `src/directx_atlas.rs` (2026-11-14): the subpixel glyph atlas is
  `B8G8R8A8_UNORM`, not `R8G8B8A8_UNORM`. Its tiles are written in BGRA
  order (`RasterizedGlyphFormat::BgraSubpixelMask`, from both the
  DirectWrite and the swash rasterizer), so the shader read the blue
  coverage as red and ClearType text came out with mirrored fringes (red on
  a glyph's left edge, blue on its right) on an RGB panel. Measured against
  Chromium's LCD text in Big Picture: the edges now carry the same colours
  as Electron's. Only windows that draw subpixel text see it (Big Picture's
  LCD rule; the main window is grayscale).

- `src/directx_renderer.rs` (2026-11-27): draws
  `SurfaceSource::WindowsVideo`. The shared texture opens once per id
  (`OpenSharedResource1`, an R8 view of the Y plane and an R8G8 view of the
  CbCr plane) and closes 32 frames after it was last drawn, since a player
  rotates a small pool. Each draw holds the keyed mutex (50 ms at most; a
  frame still being written is skipped) and passes the frame's color space
  and corner radii.
- `src/directx_renderer.rs` (2026-11-27): a blur's downsample and gaussian
  passes run under a scissor rectangle, the half-resolution region its
  composite reads (the composite box, dilated by twice the kernel's reach,
  inside the content mask), not over the whole window. Each backdrop or
  content blur used to downsample and blur the entire window; with the
  scissor the passes no longer clear their targets, since they overwrite
  every texel the next pass reads.

## gpui_ce_wgpu 0.3.0 (`../gpui-ce-wgpu`)

The crates.io release copied byte for byte from the cargo registry (src,
benches, examples, tests, Cargo.toml, LICENSE-APACHE: the manifest names its
example, test and bench targets), used through `[patch.crates-io]`
(2026-11-15). Same license (Apache-2.0). The renderer of Linux, FreeBSD and
the web; Windows doesn't compile it (DirectX), macOS uses Metal. Then
`../gpui-ce-wgpu.hydra.patch` applied on top (from the repository root:
`git apply --recount --directory=vendor/gpui-ce-wgpu
vendor/gpui-ce-wgpu.hydra.patch`; run from the crate's folder,
`git apply` skips every path silently), plus:

- `src/wgpu_renderer/filters.rs` (`CompositeExtras`,
  `blur_and_composite_group`), `src/wgpu_renderer/frame.rs`
  (`RenderCommand::EndFilter`): an isolated content-filter group composites
  through its boundary's mask fade (`BlurUniforms::with_edge_fade`, mask
  groups) and its transform (`with_transform`, transform groups), as
  `../gpui-ce-windows`' `dx_blur_and_composite` does. Backdrop filters and
  every other caller keep `blur_and_composite` unchanged.
- `src/wgpu_renderer/settings.rs` (`publish_grayscale_correction`, from
  `RenderingParameters::new`) and `src/wgpu_renderer.rs`
  (`set_font_rasterization_settings`): the monochrome sprite shader's
  grayscale gamma ratios and enhanced contrast go to
  `gpui::set_chromium_grayscale_shader_correction`, so the swash rasterizer
  can undo them for Chromium's grayscale text.

With these, `Window::with_element_mask_group` and
`with_element_transform_group` fall back only on macOS (crates.io's Metal
renderer).

## Window control areas survive a cached view's paint reuse

`src/window.rs`: `PaintIndex` gains a `window_control_hitboxes_index` lane
(recorded in `paint_index`, checked in `within`, shifted in `rebased`), and
`reuse_paint` copies the reused range of `window_control_hitboxes` into the
next frame. Window control areas (the title bar's drag area and the caption
buttons) are pushed while painting, but a cached view whose paint was replayed
never pushed them again, so a frame that reused the title bar dropped them:
the bar stopped dragging (and lost snap layouts) whenever the page under it
redrew while the bar didn't, e.g. the Home page animating its stars and flame.

The hit-test callback also tests the cursor's current position
(`platform_window.mouse_position()`) instead of `mouse_hit_test`, which keeps
the last client position once Windows sends non-client moves over the caption.

## Pixel-exact SVG masks (opt-in)

`src/window.rs` (`paint_svg_at`) and `src/elements/svg.rs`
(`Svg::pixel_exact`): `paint_svg` rasterizes a monochrome SVG at
`SMOOTH_SVG_SCALE_FACTOR` (2x) the device size and draws the tile scaled down,
which softens every edge. `paint_svg_at` takes the raster scale, and
`svg().pixel_exact()` rasterizes at 1x, the way Chromium rasterizes an inline
SVG (Big Picture's icons are compared pixel for pixel with Electron).
`paint_svg` and every existing `svg()` keep the 2x default.

## Chromium pixel snapping (opt-in, per window)

`src/taffy.rs` (`TaffyLayoutEngine::chromium_snapping`, `snap`,
`round_half_up`) and `src/window.rs` (`Window::set_chromium_pixel_snapping`):
GPUI snaps layout edges and text origins with ties toward zero, so a box
centered at y = 17.5 lands on 17. Chromium's `LayoutUnit::Round` rounds ties
up (`floor(x + 0.5)`), so the same text sits 1px lower in Electron. A window
that turns this on snaps ties up; every other window keeps GPUI's rounding.
Big Picture turns it on. In such a window a measured size (a text's
advance width) ceils to 1/64 of a device pixel, Chromium's
`LayoutUnit::FromFloatCeil`, instead of to a whole pixel
(`src/taffy.rs`, `snap_measured_size_to_layout_units`): the header's back
button is 131.19 px wide as in Electron, not 132, and what follows a text
keeps its fractional position until the edges snap.
In such a window, text truncated at its end also keeps the whitespace and
punctuation before the ellipsis (`src/elements/text.rs`,
`KEEP_BEFORE_ELLIPSIS`, or `util::CHROMIUM_SNAPPING` for a div's own inline
text, which truncates while the window draws), as Chromium's
`text-overflow: ellipsis` does ("Property …", not "Property…").
While such a window draws, authored lengths (sizes, padding, line heights
given in px) also round half up before layout (`src/util.rs`
`CHROMIUM_SNAPPING`, set for the duration of `Window::draw`).


Line heights (`src/style.rs` `line_height_in_pixels`, `src/elements/text.rs`
the text element's line height): GPUI rounds a computed line-height to a
whole device pixel (21.6px is 22, 26.4px is 26). Chromium keeps it as a
LayoutUnit, 1/64px floored (21.59375, 26.390625), so a paragraph's third
line and every box under a 26.4px heading sat up to a pixel off. In a
Chromium-snapping window the line height is floored to 1/64px instead.

Baselines (`src/text_system/line_layout.rs` `align_inline_boxes`,
`src/text_system/line.rs` `chromium_baseline`): GPUI centred the font's
unrounded ascent and descent in the line box. Blink rounds both to whole
pixels and floors the leading above the text (`CalculateLeadingSpace`), then
the glyphs round where they are painted, so a 16px line of 16px text has its
baseline 13px down, not 13.54 (rounded to 14). In a Chromium-snapping window
a line without inline boxes moves its baseline there; its extent is
unchanged. Measured on the Big Picture settings titles (ink rows 153-164 on
the reference, 154-165 before, 153-164 after).

The same two rules on the other text paths (2026-11-15):
`src/elements/div/inline.rs` (a div's own paragraph: its line height was
still `pixel_snap`ped, 19.6px became 20, so every 1.4 description was 0.4px
taller than Chromium's and its glyphs a row low; now floored to 1/64px as
the text element's), `../gpui-ce-parley/src/text_system.rs` (a parley line
that `align_inline_boxes` doesn't touch takes `chromium_baseline` of its
own height) and `src/text_system/line.rs` `paint_visual_text` (a
`ShapedText` / `ShapedLine` painted with Blink's baseline). Measured on the
Big Picture settings (every description and checkbox label inks on the
reference's rows; the language button at 491 for 490.97, the first checkbox
at 627 for 626.56; settings-general 1.93% to 1.15%, integrations 2.10% to
1.04% and its page now scrolls the reference's 84 px).

Fractional positions (2026-11-15): in such a window text paints at its
unsnapped x (`src/taffy.rs` `parent_relative_layout_bounds` /
`unsnapped_origin`, and `src/window.rs` `text_layout_bounds` for a div's
inline paragraphs, `src/elements/div/inline.rs`), as Blink paints a text
fragment at its LayoutUnit offset with subpixel glyph positions; boxes
still snap. `src/text_system/line.rs` `paint_inline_layout` keeps that x
too (2026-11-15: it snapped a div paragraph's anchor to the pixel grid
again, so the Big Picture settings' text, at x .43, drew up to half a pixel
off the reference's: emulation-detail-ps2 1.23% to 0.28%); only y snaps. `Window::unsnapped_prepaint_bounds()` gives the element being
prepainted its box before snapping (`src/element.rs` records the node in
`Window::prepaint_layout_id` around each prepaint; `src/taffy.rs`
`unsnapped_bounds`): what `getBoundingClientRect()` gives in Chromium, for
the app's focus navigation. Lengths the app authors are still rounded
before layout, so a box sized by an authored fractional length has a
whole size here.

## Chromium's LCD text rule (opt-in, per window)

`src/window.rs` (`Window::set_chromium_lcd_text`, `with_compositing_layer`,
`with_grayscale_text`, `with_composited_element_opacity`,
`note_opaque_backdrop`, `chromium_lcd_glyph`, and `paint_glyph` passing the
glyph origin to `should_use_subpixel_rendering`), `src/scene.rs`
(`LcdLayer`, `push_lcd_layer` / `pop_lcd_layer`, `note_lcd_opaque`,
`is_on_lcd_opaque`, `push_composited_opacity`, `lcd_opacity`), `src/style.rs`
(`Style::compositing_layer`, `grayscale_text`, `composited_opacity`; the
background quad notes itself opaque), `src/styled.rs` (the three builders) and
`src/color.rs` (`Background::is_opaque`).

GPUI picks one antialiasing mode per app (`TextRenderingMode`). Chromium picks
it per composited layer: LCD (ClearType) text where the text is rastered over
an opaque background of its own layer, with no opacity, filter or
non-translating transform on the way; grayscale elsewhere. A window that turns
this on (Big Picture) overrides the app's mode for its own glyphs:

- A glyph is LCD when its em box lies inside a box painted opaque earlier in
  the current layer, and grayscale otherwise. Opaque boxes are element
  backgrounds that are a solid color or an all-opaque gradient (inset by
  `r(1 - 1/√2)` for rounded corners, clipped by the content mask), painted at
  full opacity outside filters and under at most a translation; a canvas can
  add its own with `note_opaque_backdrop`.
- Layers: the window is the root one. `.compositing_layer()` starts a new one
  (its own background counts inside it, as Chromium paints a scroller's
  background in its scrolled contents); a scroll container (`overflow:
  scroll` on either axis) is always one, as Chromium composites scrollers; an
  element with a backdrop filter keeps all its text grayscale (an effect on its
  subtree), and so does `.grayscale_text()` (for the surfaces Chromium rasters
  while they animate in and doesn't raster again: toasts, popup menus).
- An element opacity below 1 turns the text inside grayscale (Chromium rasters
  the faded content into a transparent layer), unless the element is
  `.composited_opacity()` (an element with a composited descendant, such as a
  spinning icon: Chromium rasters it opaque and fades the layer). Text in an
  opacity group or a filter, or under a rotation or scale, stays grayscale as
  before.

The main window keeps the app's grayscale mode. `HYDRA_BP_LCD=0` turns it off
in Big Picture to compare. Numbers per surface in `docs/parity-big-picture.md`
(`## shell`, "Text antialiasing").

## Chromium's grayscale text (opt-in, per window; Windows, Linux)

`src/window.rs` (`Window::set_chromium_grayscale_text`, `paint_glyph`),
`src/text_system/raster.rs` (`chromium_grayscale_raster_style`,
`chromium_grayscale_color`, `chromium_preblend_table`,
`chromium_text_luminance`, `set_chromium_text_preblend` /
`chromium_text_preblend`) and `../gpui-ce-windows/src/direct_write.rs`
(`rasterize_chromium_grayscale`, `undo_grayscale_correction`; the glyph run
analysis takes ClearType antialiasing for such a glyph).

Chromium draws grayscale text with Skia's DirectWrite scaler: the A8 mask is
the luminance of the ClearType 3x1 texture (`RGBToA8`, `SkComputeLuminance`),
not DirectWrite's grayscale texture, and it goes through Skia's preblend
(`SkTMaskGamma<3, 3, 3>`, `SkTMaskGamma_build_correcting_lut`) for the text
colour's luminance. GPUI's grayscale mask is DirectWrite's own, corrected in
the sprite shader with DirectWrite's gamma and grayscale contrast: Big
Picture's toast text had 10-14% less ink than Electron's, and 133 pixels lit
on the reference were empty in Neo (stems on whole columns).

A window that turns this on asks for its grayscale glyphs as
`Grayscale` + `RasterColorEffect::Preblend(text rgb)` (the colour joins the
atlas key; no new field anywhere). The Windows rasterizer then builds the mask
as Skia does and undoes the shader's correction for that exact colour (a
256-entry inverse of `apply_contrast_and_gamma_correction` with the
renderer's `FontInfo`), so the screen gets Skia's coverage. Subpixel (LCD)
glyphs are untouched. Contrast and gamma are process-wide
(`set_chromium_text_preblend`, default 0.5 / 2.5). The gamma was fitted on
the reference's toast title (white) and description (206 at opacity 0.65):
1.8 / 2.2 / 2.5 / sRGB gave toast ink 0.93 / 0.98 / 1.01 / 0.98 of the
reference's, per-pixel error 0.056 / 0.033 / 0.029 / 0.034; the contrast
only acts on dark text and wasn't fitted (Skia's default). Platforms whose
rasterizer doesn't know this style draw their usual grayscale mask.

The swash rasterizer (Linux; 2026-11-15): `../gpui-ce-parley/src/store.rs`
(`chromium_grayscale_mask`) runs swash's A8 mask through the same preblend
and undoes the wgpu sprite shader's correction, which the wgpu renderer
publishes (`set_chromium_grayscale_shader_correction`; `ZED_FONTS_GAMMA`
1.8 and grayscale contrast 1.0 unless set). The undo is shared
(`chromium_undo_grayscale_correction`, also behind the DirectWrite one).
Off Windows the preblend defaults to the identity (contrast 0, gamma 1):
Chromium's Linux Skia only preblends a FreeType A8 mask under
`SK_GAMMA_APPLY_TO_A8`, which Chromium's build defines for Android alone, so
the screen gets FreeType's own coverage. Not checked against a Linux
Electron yet; swash's hinting isn't FreeType's either. Without a published
correction (swash as the CBDT fallback under DirectX) the mask is left as
it was.

LCD glyphs (2026-11-15, Windows only): `Window::set_chromium_subpixel_preblend`
asks for subpixel glyphs as `Subpixel` + `RasterColorEffect::Preblend(text
rgb)` (`chromium_subpixel_raster_style` / `chromium_subpixel_color`); the
DirectWrite rasterizer (`rasterize_chromium_subpixel`) runs each of the
ClearType texture's three coverages through the preblend table for that
channel of the colour (Skia's `RGBToLcd16<APPLY_PREBLEND>` with
`SkTMaskGamma::preBlend`'s R, G and B tables, the same contrast / gamma) and
undoes the subpixel sprite shader's correction per channel
(`chromium_undo_subpixel_correction`: the dark-text contrast from the
colour's luma, the gamma correction from the channel's value). Big
Picture's game description had 8.5% less ink than Electron's (signed luma
-10.4 on ink pixels); with it, 2.7% more (+3.2), and `game-steam@900` went
1.82% to 0.18% against a fresh reference capture. The swash rasterizer
doesn't know the style: turn it on only where DirectWrite rasterizes.

## Text shadows (opt-in, per element)

CSS `text-shadow` (one shadow), for Big Picture's modal, sidebar-modal and
notification titles, the profile copy and the library list's title
fallback. Without a shadow nothing changes.

- `src/style.rs`: `TextShadow { offset, blur, color }` (`TextShadow::new(x,
  y, blur, color)`), and `TextStyle::text_shadow: Option<TextShadow>`
  (default None), which cascades like the other text properties.
- `src/styled.rs`: `Styled::text_shadow(TextShadow)`.
- `src/window.rs`: `Window::paint_text_shadow(bounds, paint)`: when the
  current text style has a shadow, `paint` runs first with the shadow's
  offset, inside a content filter layer (`with_filter_layer`, the `filter:
  blur()` group) whose standard deviation is half the CSS blur radius (as
  Chromium's `BlurRadiusToStdDev`), with `Window::text_color_override` set
  so every glyph takes the shadow's color; color glyphs (emoji) are left
  out of the shadow pass. A zero blur is a plain offset copy (no layer).
- `src/elements/text.rs` (`TextLayout::paint_lines`) and
  `src/elements/div/inline.rs` (`paint_paragraphs`, which now records each
  paragraph's size in prepaint) paint the shadow pass before the glyphs.

Cost: one offscreen layer per shadowed text element per frame (the
renderers' existing blur path). Text inside the layer rasterizes in
grayscale, which a blurred shadow hides. Decorations (underline,
strikethrough) get no shadow.

## Windows opened at off-display bounds (opt-in, Windows)

`src/platform.rs` (`WindowOptions::keep_given_bounds`, and
`WindowParams::keep_given_bounds` under `cfg(target_os = "windows")` so the
other platforms' crates.io backends still build their `WindowParams`),
`src/window.rs` (passes it on) and `../gpui-ce-windows/src/window.rs`
(`WindowsWindow::new`, `retrieve_window_placement`): GPUI replaced windowed
bounds whose centre is on no display with the display's default bounds
(`check_given_bounds`), and `SetWindowPlacement` itself moves a window that
would be entirely off screen back onto one. Big Picture's background test
window (`HYDRA_BP_SIZE` with `--hidden`) sits just left of the desktop, so it
first appeared centred on the user's screen at 1552x1103 until a helper moved
it. With the option set, the given bounds are kept and the window, created
without `WS_VISIBLE`, is shown once with `SetWindowPos` (`SWP_SHOWWINDOW`,
plus `SWP_NOACTIVATE` when it doesn't take the focus) already at them.
Default off; every other window keeps GPUI's check.

## gpui_ce_parley 0.3.0 (`../gpui-ce-parley`)

The crates.io release (src, Cargo.toml without its bench target, the
Apache-2.0 license file copied from gpui-ce-windows), used through
`[patch.crates-io]` (2026-11-14).

- `src/text_system.rs` (the line metrics in the layout pass): a
  `line-height` under the font's own height packs the lines at that height,
  as CSS does (a negative half-leading; the glyphs overflow the line box).
  GPUI clamped the half-leading at zero, so a 15px line-height on 14px
  Space Grotesk (17.86px tall) still gave 18px lines and a 40px line on
  32px text 41px. Opt-in: only while a window that snaps like Chromium
  draws (`gpui::chromium_snapping_active()`, a new public reader of the
  flag in `src/util.rs`, re-exported from `src/gpui.rs`). Line layouts are
  cached by their text and style, not by this flag; Big Picture is the
  only such window and the main window is hidden while it is open.
- `src/text_system.rs` (the paragraph builder, every window): the bidi
  controls `prepare_bidi_text` inserts (the root LRM / RLM before every
  text with a resolved direction, the isolates around spans) get a letter
  spacing of 0. Parley adds `letter-spacing` to every cluster, so each
  control widened the text by one spacing and moved its glyphs right by it:
  "LB" at 11px with 0.44px spacing measured 14.59 against Chromium's 14.16
  (the settings tab row 0.88px wider, its content 0.44px left). Text
  without letter spacing is unchanged (2026-11-15).
- `src/text_system.rs` (the paragraph builder, every window): every
  paragraph gets `overflow-wrap: break-word`. Parley's default (`normal`)
  never split a word, so a long unbroken run ("aaaa...", a URL) in a
  wrapping text area or label ran past its right edge on one line. With
  `break-word` a word only splits when it alone can't fit the line, so text
  that already fit lays out the same. Parley takes break-word opportunities
  even at a zero line width, which is how GPUI measures min-content, so a
  zero wrap width breaks at the paragraph's min-content width
  (`calculate_content_widths().min`, which ignores break-word as CSS does)
  instead of after every character (2026-11-23).
