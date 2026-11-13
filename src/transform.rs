use serde::{Deserialize, Serialize};

/// Legacy PI constant from `illustrate.f` for exact floating-point parity.
#[allow(clippy::approx_constant)]
pub const LEGACY_PI: f32 = 3.141592;

/// 4x4 transformation matrix matching Fortran `real*4 m(4,4)` row/column layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Matrix4 {
    /// Elements stored in `[row][col]` order (0-indexed).
    pub data: [[f32; 4]; 4],
}

impl Default for Matrix4 {
    fn default() -> Self {
        Self::identity()
    }
}

impl Matrix4 {
    /// Returns the 4x4 identity matrix (`clearmatrix` in `illustrate.f`).
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            data: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Builds an X-axis rotation matrix using the legacy left-handed sign convention.
    #[must_use]
    pub fn rotation_x(degrees: f32) -> Self {
        let angle = -degrees * LEGACY_PI / 180.0;
        let mut m = Self::identity();
        m.data[1][1] = angle.cos();
        m.data[1][2] = -angle.sin();
        m.data[2][1] = angle.sin();
        m.data[2][2] = angle.cos();
        m
    }

    /// Builds a Y-axis rotation matrix using the legacy left-handed sign convention.
    #[must_use]
    pub fn rotation_y(degrees: f32) -> Self {
        let angle = -degrees * LEGACY_PI / 180.0;
        let mut m = Self::identity();
        m.data[0][0] = angle.cos();
        m.data[0][2] = angle.sin();
        m.data[2][0] = -angle.sin();
        m.data[2][2] = angle.cos();
        m
    }

    /// Builds a Z-axis rotation matrix using the legacy left-handed sign convention.
    #[must_use]
    pub fn rotation_z(degrees: f32) -> Self {
        let angle = -degrees * LEGACY_PI / 180.0;
        let mut m = Self::identity();
        m.data[0][0] = angle.cos();
        m.data[0][1] = -angle.sin();
        m.data[1][0] = angle.sin();
        m.data[1][1] = angle.cos();
        m
    }

    /// Concatenates `other` onto `self` matching Fortran `subroutine catenate(m1, m2)`.
    ///
    /// Note that `catenate` computes `m[i][j] = sum_k m1[k][j] * m2[i][k]`, which
    /// causes sequentially added rotation cards to apply effectively last-to-first.
    pub fn catenate(&mut self, other: &Self) {
        let mut out = [[0.0_f32; 4]; 4];
        for (i, out_row) in out.iter_mut().enumerate() {
            for (j, cell) in out_row.iter_mut().enumerate() {
                *cell = self.data[0][j] * other.data[i][0]
                    + self.data[1][j] * other.data[i][1]
                    + self.data[2][j] * other.data[i][2]
                    + self.data[3][j] * other.data[i][3];
            }
        }
        self.data = out;
    }

    /// Applies the 3x3 rotation portion of `self` using Fortran's transposed vector-matrix layout.
    #[must_use]
    #[inline]
    pub fn transform_vector(&self, v: [f32; 3]) -> [f32; 3] {
        [
            v[0] * self.data[0][0] + v[1] * self.data[1][0] + v[2] * self.data[2][0],
            v[0] * self.data[0][1] + v[1] * self.data[1][1] + v[2] * self.data[2][1],
            v[0] * self.data[0][2] + v[1] * self.data[1][2] + v[2] * self.data[2][2],
        ]
    }

    /// Constructs a `Matrix4` from `[xrot, yrot, zrot]` Euler angles in degrees, matching
    /// canonical `zrot` -> `yrot` -> `xrot` card order (`X` applied first, then `Y`, then `Z`).
    #[must_use]
    pub fn from_euler_angles_deg(xrot: f32, yrot: f32, zrot: f32) -> Self {
        let mut m = Self::identity();
        if zrot != 0.0 {
            m.catenate(&Self::rotation_z(zrot));
        }
        if yrot != 0.0 {
            m.catenate(&Self::rotation_y(yrot));
        }
        if xrot != 0.0 {
            m.catenate(&Self::rotation_x(xrot));
        }
        m
    }

    /// Decomposes the 3x3 rotation portion into `[xrot, yrot, zrot]` degrees (`-180..=180`).
    #[must_use]
    pub fn to_euler_angles_deg(&self) -> [f32; 3] {
        let m = &self.data;
        let sb = m[0][2].clamp(-1.0, 1.0);
        let beta = sb.asin();
        let cb = beta.cos();

        let (alpha, gamma) = if cb.abs() > 1e-5 {
            ((-m[1][2]).atan2(m[2][2]), (-m[0][1]).atan2(m[0][0]))
        } else {
            (m[2][1].atan2(m[1][1]), 0.0)
        };

        let to_deg = |rad: f32| {
            let d = -rad * 180.0 / LEGACY_PI;
            if d.abs() < 1e-5 { 0.0 } else { d }
        };
        [to_deg(alpha), to_deg(beta), to_deg(gamma)]
    }
}

/// 3x4 affine transformation matrix for PDB `REMARK 350 BIOMT` biological assemblies.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BioMatrix {
    /// 3 rows x 4 columns (`[row][0..3]` rotation, `[row][3]` translation).
    pub rows: [[f32; 4]; 3],
}

impl Default for BioMatrix {
    fn default() -> Self {
        Self::identity()
    }
}

impl BioMatrix {
    /// Returns the 3x4 identity affine transform.
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            rows: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        }
    }

    /// Transforms an atomic coordinate `[x, y, z]` by this biological assembly matrix.
    #[must_use]
    #[inline]
    pub fn apply(&self, coord: [f32; 3]) -> [f32; 3] {
        [
            coord[0] * self.rows[0][0]
                + coord[1] * self.rows[0][1]
                + coord[2] * self.rows[0][2]
                + self.rows[0][3],
            coord[0] * self.rows[1][0]
                + coord[1] * self.rows[1][1]
                + coord[2] * self.rows[1][2]
                + self.rows[1][3],
            coord[0] * self.rows[2][0]
                + coord[1] * self.rows[2][1]
                + coord[2] * self.rows[2][2]
                + self.rows[2][3],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn z_rotation_90_matches_legacy_left_handed_convention() {
        let mut rm = Matrix4::identity();
        rm.catenate(&Matrix4::rotation_z(90.0));
        let out = rm.transform_vector([1.0, 0.0, 0.0]);
        assert!(out[0].abs() < 1e-6);
        assert!((out[1] - 1.0).abs() < 1e-6);
        assert!(out[2].abs() < 1e-6);
    }

    #[test]
    fn sequential_rotations_apply_last_to_first() {
        let mut rm = Matrix4::identity();
        rm.catenate(&Matrix4::rotation_z(90.0));
        rm.catenate(&Matrix4::rotation_x(90.0));

        // Verify exact equivalence to applying rotation_x(90) then rotation_z(90)
        let combined = rm.transform_vector([1.0, 2.0, 3.0]);
        let step1 = Matrix4::rotation_x(90.0).transform_vector([1.0, 2.0, 3.0]);
        let step2 = Matrix4::rotation_z(90.0).transform_vector(step1);

        for i in 0..3 {
            assert!((combined[i] - step2[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn euler_angles_round_trip_preserves_rotation_matrix() {
        let cases = [
            [0.0, 0.0, 90.0],
            [25.0, -40.0, 110.0],
            [-15.0, 89.9, -45.0],
            [35.0, 90.0, 0.0],
        ];
        for [xr, yr, zr] in cases {
            let m = Matrix4::from_euler_angles_deg(xr, yr, zr);
            let [xr2, yr2, zr2] = m.to_euler_angles_deg();
            let m2 = Matrix4::from_euler_angles_deg(xr2, yr2, zr2);
            let test_vec = [1.3, -2.7, 4.1];
            let v1 = m.transform_vector(test_vec);
            let v2 = m2.transform_vector(test_vec);
            for i in 0..3 {
                assert!(
                    (v1[i] - v2[i]).abs() < 1e-4,
                    "mismatch for [{xr}, {yr}, {zr}] -> [{xr2}, {yr2}, {zr2}]"
                );
            }
        }
    }
}
