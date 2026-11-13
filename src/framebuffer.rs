/// Border padding matching Fortran's `-10:3008` frame buffer guard bands.
pub const FRAME_PADDING: i32 = 10;

/// Dynamically allocated, padded 2D frame buffer holding depth, atom/bio IDs, and RGBA values.
///
/// Coordinates follow the legacy convention:
/// - `ix` (`1..=ixsize`): vertical row index (`+x` down)
/// - `iy` (`1..=iysize`): horizontal column index (`+y` left to right)
/// - Guard band `(-FRAME_PADDING..=size + FRAME_PADDING)` is initialized to `0` (`zpix = 0.0`,
///   `atom = 0`, `bio = 0`) so border outline stencils never panic on out-of-bounds reads.
#[derive(Debug, Clone)]
pub struct FrameBuffer {
    /// Active vertical size (`ixsize`).
    pub ixsize: i32,
    /// Active horizontal size (`iysize`).
    pub iysize: i32,
    stride: usize,
    /// Depth buffer (`zpix`).
    zpix: Vec<f32>,
    /// Closest atom index (`0` = background sentinel).
    atom: Vec<usize>,
    /// Biological assembly matrix index (`1`-based inside active frame, `0` in guard band).
    bio: Vec<i32>,
    /// RGBA float color buffer (`pix`).
    pix: Vec<[f32; 4]>,
}

impl FrameBuffer {
    /// Allocates a padded frame buffer for an image of `ixsize` rows by `iysize` columns.
    #[must_use]
    pub fn new(ixsize: i32, iysize: i32) -> Self {
        let rows = (ixsize + 2 * FRAME_PADDING + 1).max(1) as usize;
        let cols = (iysize + 2 * FRAME_PADDING + 1).max(1) as usize;
        let total = rows * cols;

        let mut fb = Self {
            ixsize,
            iysize,
            stride: cols,
            zpix: vec![0.0_f32; total],
            atom: vec![0; total],
            bio: vec![0; total],
            pix: vec![[0.0_f32; 4]; total],
        };

        for ix in 1..=ixsize {
            for iy in 1..=iysize {
                let idx = fb.index(ix, iy);
                fb.zpix[idx] = -10_000.0_f32;
                fb.bio[idx] = 1;
            }
        }

        fb
    }

    #[inline]
    fn index(&self, ix: i32, iy: i32) -> usize {
        debug_assert!(
            ix >= -FRAME_PADDING && ix <= self.ixsize + FRAME_PADDING,
            "ix={ix} out of padded range"
        );
        debug_assert!(
            iy >= -FRAME_PADDING && iy <= self.iysize + FRAME_PADDING,
            "iy={iy} out of padded range"
        );
        let r = (ix + FRAME_PADDING) as usize;
        let c = (iy + FRAME_PADDING) as usize;
        r * self.stride + c
    }

    /// Reads depth `zpix(ix, iy)`.
    #[must_use]
    #[inline]
    pub fn z(&self, ix: i32, iy: i32) -> f32 {
        self.zpix[self.index(ix, iy)]
    }

    /// Writes depth `zpix(ix, iy)`.
    #[inline]
    pub fn set_z(&mut self, ix: i32, iy: i32, val: f32) {
        let idx = self.index(ix, iy);
        self.zpix[idx] = val;
    }

    /// Reads atom index `atom(ix, iy)`.
    #[must_use]
    #[inline]
    pub fn atom(&self, ix: i32, iy: i32) -> usize {
        self.atom[self.index(ix, iy)]
    }

    /// Writes atom index `atom(ix, iy)`.
    #[inline]
    pub fn set_atom(&mut self, ix: i32, iy: i32, val: usize) {
        let idx = self.index(ix, iy);
        self.atom[idx] = val;
    }

    /// Reads biological assembly index `bio(ix, iy)`.
    #[must_use]
    #[inline]
    pub fn bio(&self, ix: i32, iy: i32) -> i32 {
        self.bio[self.index(ix, iy)]
    }

    /// Writes biological assembly index `bio(ix, iy)`.
    #[inline]
    pub fn set_bio(&mut self, ix: i32, iy: i32, val: i32) {
        let idx = self.index(ix, iy);
        self.bio[idx] = val;
    }

    /// Reads RGBA color tuple `pix(ix, iy, 1..4)`.
    #[must_use]
    #[inline]
    pub fn pixel(&self, ix: i32, iy: i32) -> [f32; 4] {
        self.pix[self.index(ix, iy)]
    }

    /// Writes RGBA color tuple `pix(ix, iy, 1..4)`.
    #[inline]
    pub fn set_pixel(&mut self, ix: i32, iy: i32, rgba: [f32; 4]) {
        let idx = self.index(ix, iy);
        self.pix[idx] = rgba;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_band_and_active_region_defaults_match_fortran() {
        let fb = FrameBuffer::new(10, 12);
        assert_eq!(fb.z(1, 1), -10_000.0);
        assert_eq!(fb.bio(1, 1), 1);
        assert_eq!(fb.atom(1, 1), 0);

        // Guard band outside 1..=size is zero-initialized
        assert_eq!(fb.z(0, 0), 0.0);
        assert_eq!(fb.bio(0, 0), 0);
        assert_eq!(fb.z(11, 13), 0.0);
    }
}
