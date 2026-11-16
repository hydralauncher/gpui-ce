//! Hydra patch: element transforms and edge masks, the CSS `transform` /
//! `transform-origin` and `mask-image: linear-gradient(..)` of an element and
//! everything it paints. See `HYDRA-PATCHES.md`.

use crate::{
    AbsoluteLength, Bounds, ContentMask, DefiniteLength, Edges, Pixels, Point, Radians,
    TransformationMatrix, point, px, relative,
};
use serde::{Deserialize, Serialize};

/// A 2D transform of an element and its children, like CSS `transform`.
///
/// Set it with [`crate::Styled::transform`]. Layout ignores it, as in CSS:
/// the element keeps its box, and everything it paints (background, border,
/// shadows, text, images, SVGs, paths, children) is drawn through the
/// transform around [`ElementTransform::origin`]. Hit testing follows it.
///
/// Build it like a CSS transform list: the constructors start one, the `then_`
/// methods append a function, and later functions apply to a point first, so
/// `translate(..) rotate(..)` in CSS is
/// `ElementTransform::translate(..).then_rotate(..)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementTransform {
    /// The linear part, row-major: `[[a, c], [b, d]]` for CSS
    /// `matrix(a, b, c, d, e, f)`.
    pub linear: [[f32; 2]; 2],
    /// The translation in logical pixels (CSS `e`, `f`).
    pub translation: Point<Pixels>,
    /// The point the transform is applied around (CSS `transform-origin`),
    /// from the element's top left; fractions are of the element's size. The
    /// default is the center.
    pub origin: Point<DefiniteLength>,
}

impl Default for ElementTransform {
    fn default() -> Self {
        Self {
            linear: [[1.0, 0.0], [0.0, 1.0]],
            translation: Point::default(),
            origin: point(relative(0.5).into(), relative(0.5).into()),
        }
    }
}

impl ElementTransform {
    /// The identity transform.
    pub fn identity() -> Self {
        Self::default()
    }

    /// CSS `scale(factor)`.
    pub fn scale(factor: f32) -> Self {
        Self::identity().then_scale_xy(factor, factor)
    }

    /// CSS `scale(x, y)`.
    pub fn scale_xy(x: f32, y: f32) -> Self {
        Self::identity().then_scale_xy(x, y)
    }

    /// CSS `rotate(angle)`: clockwise on screen.
    pub fn rotate(angle: impl Into<Radians>) -> Self {
        Self::identity().then_rotate(angle)
    }

    /// CSS `rotate(<degrees>deg)`.
    pub fn rotate_degrees(degrees: f32) -> Self {
        Self::rotate(Radians(degrees.to_radians()))
    }

    /// CSS `translate(x, y)`.
    pub fn translate(x: Pixels, y: Pixels) -> Self {
        Self::identity().then_translate(x, y)
    }

    /// Appends CSS `scale(x, y)` (applied to points before this transform).
    pub fn then_scale_xy(self, x: f32, y: f32) -> Self {
        self.then_matrix([[x, 0.0], [0.0, y]], Point::default())
    }

    /// Appends CSS `scale(factor)`.
    pub fn then_scale(self, factor: f32) -> Self {
        self.then_scale_xy(factor, factor)
    }

    /// Appends CSS `rotate(angle)`.
    pub fn then_rotate(self, angle: impl Into<Radians>) -> Self {
        let angle = angle.into().0;
        let (sin, cos) = angle.sin_cos();
        self.then_matrix([[cos, -sin], [sin, cos]], Point::default())
    }

    /// Appends CSS `rotate(<degrees>deg)`.
    pub fn then_rotate_degrees(self, degrees: f32) -> Self {
        self.then_rotate(Radians(degrees.to_radians()))
    }

    /// Appends CSS `translate(x, y)`.
    pub fn then_translate(self, x: Pixels, y: Pixels) -> Self {
        self.then_matrix([[1.0, 0.0], [0.0, 1.0]], point(x, y))
    }

    /// Sets CSS `transform-origin` (lengths or fractions of the element's size).
    pub fn origin(mut self, x: impl Into<DefiniteLength>, y: impl Into<DefiniteLength>) -> Self {
        self.origin = point(x.into(), y.into());
        self
    }

    /// Whether this transform leaves every point where it is.
    pub fn is_identity(&self) -> bool {
        self.linear == [[1.0, 0.0], [0.0, 1.0]] && self.translation == Point::default()
    }

    fn then_matrix(self, linear: [[f32; 2]; 2], translation: Point<Pixels>) -> Self {
        let a = self.linear;
        let product = [
            [
                a[0][0] * linear[0][0] + a[0][1] * linear[1][0],
                a[0][0] * linear[0][1] + a[0][1] * linear[1][1],
            ],
            [
                a[1][0] * linear[0][0] + a[1][1] * linear[1][0],
                a[1][0] * linear[0][1] + a[1][1] * linear[1][1],
            ],
        ];
        let (tx, ty) = (translation.x.0, translation.y.0);
        Self {
            linear: product,
            translation: point(
                self.translation.x + px(a[0][0] * tx + a[0][1] * ty),
                self.translation.y + px(a[1][0] * tx + a[1][1] * ty),
            ),
            origin: self.origin,
        }
    }

    /// The transform of an element laid out at `bounds`, in window coordinates
    /// (logical pixels): around the resolved origin.
    pub fn to_matrix(&self, bounds: Bounds<Pixels>, rem_size: Pixels) -> TransformationMatrix {
        let origin = bounds.origin
            + point(
                self.origin
                    .x
                    .to_pixels(AbsoluteLength::Pixels(bounds.size.width), rem_size),
                self.origin
                    .y
                    .to_pixels(AbsoluteLength::Pixels(bounds.size.height), rem_size),
            );
        let own = TransformationMatrix {
            rotation_scale: self.linear,
            translation: [self.translation.x.0, self.translation.y.0],
        };
        let to_origin = TransformationMatrix {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [origin.x.0, origin.y.0],
        };
        let from_origin = TransformationMatrix {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [-origin.x.0, -origin.y.0],
        };
        to_origin.compose(own).compose(from_origin)
    }
}

/// One edge's fade in an [`EdgeMask`], measured inward from that edge:
/// transparent up to `start`, opaque from `end`, linear between.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeFade {
    /// Where the fade starts (fully transparent before it).
    pub start: DefiniteLength,
    /// Where the content is fully opaque again.
    pub end: DefiniteLength,
}

/// An alpha mask over an element and its children made of linear fades at its
/// edges: CSS `mask-image: linear-gradient(..)` for edge fades. Like a CSS
/// mask, it also clips to the element's box.
///
/// Set it with [`crate::Styled::mask`]. Fractions are of the element's height
/// for the top and bottom edges and of its width for the left and right ones.
/// For example `linear-gradient(90deg, #000 30%, transparent 80%)` is
/// `EdgeMask::new().right(relative(0.2), relative(0.7))`, and
/// `linear-gradient(transparent, #000 24%, #000 76%, transparent)` is
/// `EdgeMask::new().top(px(0.), relative(0.24)).bottom(px(0.), relative(0.24))`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeMask {
    /// The fade at the top edge.
    pub top: Option<EdgeFade>,
    /// The fade at the right edge.
    pub right: Option<EdgeFade>,
    /// The fade at the bottom edge.
    pub bottom: Option<EdgeFade>,
    /// The fade at the left edge.
    pub left: Option<EdgeFade>,
}

impl EdgeMask {
    /// A mask that only clips to the element's box.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fades the top edge: transparent to `start`, opaque from `end`.
    pub fn top(mut self, start: impl Into<DefiniteLength>, end: impl Into<DefiniteLength>) -> Self {
        self.top = Some(EdgeFade {
            start: start.into(),
            end: end.into(),
        });
        self
    }

    /// Fades the right edge: transparent to `start`, opaque from `end`.
    pub fn right(
        mut self,
        start: impl Into<DefiniteLength>,
        end: impl Into<DefiniteLength>,
    ) -> Self {
        self.right = Some(EdgeFade {
            start: start.into(),
            end: end.into(),
        });
        self
    }

    /// Fades the bottom edge: transparent to `start`, opaque from `end`.
    pub fn bottom(
        mut self,
        start: impl Into<DefiniteLength>,
        end: impl Into<DefiniteLength>,
    ) -> Self {
        self.bottom = Some(EdgeFade {
            start: start.into(),
            end: end.into(),
        });
        self
    }

    /// Fades the left edge: transparent to `start`, opaque from `end`.
    pub fn left(
        mut self,
        start: impl Into<DefiniteLength>,
        end: impl Into<DefiniteLength>,
    ) -> Self {
        self.left = Some(EdgeFade {
            start: start.into(),
            end: end.into(),
        });
        self
    }

    /// The content mask of an element laid out at `bounds`.
    pub fn content_mask(&self, bounds: Bounds<Pixels>, rem_size: Pixels) -> ContentMask<Pixels> {
        let resolve = |fade: Option<EdgeFade>, length: Pixels| match fade {
            Some(fade) => {
                let base = AbsoluteLength::Pixels(length);
                let start = fade.start.to_pixels(base, rem_size);
                let end = fade.end.to_pixels(base, rem_size);
                (start, (end - start).max(Pixels::ZERO))
            }
            None => (Pixels::ZERO, Pixels::ZERO),
        };
        let (top_start, top) = resolve(self.top, bounds.size.height);
        let (right_start, right) = resolve(self.right, bounds.size.width);
        let (bottom_start, bottom) = resolve(self.bottom, bounds.size.height);
        let (left_start, left) = resolve(self.left, bounds.size.width);
        ContentMask {
            bounds,
            fade_out: Edges {
                top,
                right,
                bottom,
                left,
            },
            fade_start: Edges {
                top: top_start,
                right: right_start,
                bottom: bottom_start,
                left: left_start,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::size;

    #[test]
    fn scale_is_around_the_center_by_default() {
        let bounds = Bounds::new(point(px(100.), px(50.)), size(px(200.), px(100.)));
        let matrix = ElementTransform::scale(0.5).to_matrix(bounds, px(16.));
        assert_eq!(
            matrix.apply(point(px(200.), px(100.))),
            point(px(200.), px(100.))
        );
        assert_eq!(
            matrix.apply(point(px(100.), px(50.))),
            point(px(150.), px(75.))
        );
    }

    #[test]
    fn functions_apply_right_to_left_like_css() {
        // translate(10px, 0) scale(2) around the top left: (1, 1) -> (2, 2) -> (12, 2)
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(10.), px(10.)));
        let matrix = ElementTransform::translate(px(10.), px(0.))
            .then_scale(2.0)
            .origin(px(0.), px(0.))
            .to_matrix(bounds, px(16.));
        assert_eq!(matrix.apply(point(px(1.), px(1.))), point(px(12.), px(2.)));
        let inverse = matrix.inverse().unwrap();
        assert_eq!(inverse.apply(point(px(12.), px(2.))), point(px(1.), px(1.)));
    }

    #[test]
    fn edge_mask_resolves_fractions_per_axis() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(300.)));
        let mask = EdgeMask::new()
            .top(px(0.), relative(0.24))
            .right(relative(0.2), relative(0.7))
            .content_mask(bounds, px(16.));
        let close = |a: Pixels, b: f32| (a.0 - b).abs() < 1e-3;
        assert!(close(mask.fade_out.top, 72.));
        assert!(close(mask.fade_start.right, 80.));
        assert!(close(mask.fade_out.right, 200.));
        assert_eq!(mask.fade_out.left, px(0.));
    }
}
