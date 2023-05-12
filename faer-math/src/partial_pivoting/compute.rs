use assert2::{assert, debug_assert};
use dyn_stack::{DynStack, SizeOverflow, StackReq};
use faer_core::{
    mul::matmul,
    permutation::{swap_rows, PermutationMut},
    solve::solve_unit_lower_triangular_in_place,
    temp_mat_req, zipped, ComplexField, Entity, MatMut, Parallelism,
};
use reborrow::*;

#[inline(always)]
fn swap_two_elems<E: ComplexField>(mut m: MatMut<'_, E>, i: usize, j: usize) {
    debug_assert!(m.ncols() == 1);
    debug_assert!(i < m.nrows());
    debug_assert!(j < m.nrows());
    unsafe {
        let a = m.read_unchecked(i, 0);
        let b = m.read_unchecked(j, 0);
        m.write_unchecked(i, 0, b);
        m.write_unchecked(j, 0, a);
    }
}

#[inline(always)]
fn swap_two_elems_contiguous<E: ComplexField>(mut m: MatMut<'_, E>, i: usize, j: usize) {
    debug_assert!(m.ncols() == 1);
    debug_assert!(m.row_stride() == 1);
    debug_assert!(i < m.nrows());
    debug_assert!(j < m.nrows());
    unsafe {
        let ptr = m.rb_mut().as_ptr();

        let ptr_a = E::map(
            E::copy(&ptr),
            #[inline(always)]
            |ptr| ptr.add(i),
        );
        let ptr_b = E::map(
            E::copy(&ptr),
            #[inline(always)]
            |ptr| ptr.add(j),
        );

        let a = E::map(
            E::copy(&ptr_a),
            #[inline(always)]
            |ptr| (*ptr).clone(),
        );
        let b = E::map(
            E::copy(&ptr_b),
            #[inline(always)]
            |ptr| (*ptr).clone(),
        );

        E::map(
            E::zip(ptr_b, a),
            #[inline(always)]
            |(ptr, val)| *ptr = val,
        );
        E::map(
            E::zip(ptr_a, b),
            #[inline(always)]
            |(ptr, val)| *ptr = val,
        );
    }
}

fn lu_unblocked_req<E: Entity>(_m: usize, _n: usize) -> Result<StackReq, SizeOverflow> {
    Ok(StackReq::default())
}

#[inline(never)]
fn lu_in_place_unblocked<E: ComplexField>(
    mut matrix: MatMut<'_, E>,
    col_start: usize,
    n: usize,
    perm: &mut [usize],
    transpositions: &mut [usize],
    mut stack: DynStack<'_>,
) -> usize {
    let m = matrix.nrows();
    debug_assert!(m >= n);
    debug_assert!(perm.len() == m);

    if n == 0 {
        return 0;
    }

    let mut n_transpositions = 0;

    for (j, t) in transpositions.iter_mut().enumerate() {
        let mut max = E::Real::zero();
        let mut imax = j;

        for i in j..m {
            let abs = matrix.read(i, j + col_start).score();
            if abs > max {
                imax = i;
                max = abs;
            }
        }

        *t = imax - j;

        if imax != j {
            n_transpositions += 1;
            perm.swap(j, imax);
        }

        swap_rows(matrix.rb_mut(), j, imax);

        let [_, _, _, middle_right] = matrix.rb_mut().split_at(0, col_start);
        let [_, _, middle, _] = middle_right.split_at(0, n);
        update(middle, j, stack.rb_mut());
    }

    n_transpositions
}

fn update<E: ComplexField>(mut matrix: MatMut<E>, j: usize, _stack: DynStack<'_>) {
    let m = matrix.nrows();
    let inv = matrix.read(j, j).inv();
    for i in j + 1..m {
        matrix.write(i, j, matrix.read(i, j).mul(&inv));
    }
    let [_, top_right, bottom_left, bottom_right] = matrix.rb_mut().split_at(j + 1, j + 1);
    let lhs = bottom_left.rb().col(j);
    let rhs = top_right.rb().row(j);
    let mut mat = bottom_right;

    for k in 0..mat.ncols() {
        let col = mat.rb_mut().col(k);
        let rhs = rhs.read(0, k);
        zipped!(col, lhs).for_each(|mut x, lhs| x.write(x.read().sub(&lhs.read().mul(&rhs))));
    }
}

fn recursion_threshold<E: Entity>(_m: usize) -> usize {
    16
}

#[inline]
// we want remainder to be a multiple of register size
fn blocksize<E: Entity>(n: usize) -> usize {
    let base_rem = n / 2;
    n - if n >= 32 {
        (base_rem + 15) / 16 * 16
    } else if n >= 16 {
        (base_rem + 7) / 8 * 8
    } else if n >= 8 {
        (base_rem + 3) / 4 * 4
    } else {
        base_rem
    }
}

fn lu_recursive_req<E: Entity>(
    m: usize,
    n: usize,
    parallelism: Parallelism,
) -> Result<StackReq, SizeOverflow> {
    if n <= recursion_threshold::<E>(m) {
        return lu_unblocked_req::<E>(m, n);
    }

    let bs = blocksize::<E>(n);

    StackReq::try_any_of([
        lu_recursive_req::<E>(m, bs, parallelism)?,
        StackReq::try_all_of([
            StackReq::try_new::<usize>(m - bs)?,
            lu_recursive_req::<E>(m - bs, n - bs, parallelism)?,
        ])?,
        temp_mat_req::<E>(m, 1)?,
    ])
}

fn lu_in_place_impl<E: ComplexField>(
    mut matrix: MatMut<'_, E>,
    col_start: usize,
    n: usize,
    perm: &mut [usize],
    transpositions: &mut [usize],
    parallelism: Parallelism,
    mut stack: DynStack<'_>,
) -> usize {
    let m = matrix.nrows();
    let full_n = matrix.ncols();

    debug_assert!(m >= n);
    debug_assert!(perm.len() == m);

    if n <= recursion_threshold::<E>(m) {
        return lu_in_place_unblocked(matrix, col_start, n, perm, transpositions, stack);
    }

    let bs = blocksize::<E>(n);

    let mut n_transpositions = 0;

    n_transpositions += lu_in_place_impl(
        matrix.rb_mut().submatrix(0, col_start, m, n),
        0,
        bs,
        perm,
        &mut transpositions[..bs],
        parallelism,
        stack.rb_mut(),
    );

    let [mat_top_left, mut mat_top_right, mat_bot_left, mut mat_bot_right] = matrix
        .rb_mut()
        .submatrix(0, col_start, m, n)
        .split_at(bs, bs);

    solve_unit_lower_triangular_in_place(mat_top_left.rb(), mat_top_right.rb_mut(), parallelism);
    matmul(
        mat_bot_right.rb_mut(),
        mat_bot_left.rb(),
        mat_top_right.rb(),
        Some(E::one()),
        E::one().neg(),
        parallelism,
    );

    {
        let (mut tmp_perm, mut stack) = stack.rb_mut().make_with(m - bs, |i| i);
        let tmp_perm = &mut *tmp_perm;
        n_transpositions += lu_in_place_impl(
            matrix.rb_mut().submatrix(bs, col_start, m - bs, n),
            bs,
            n - bs,
            tmp_perm,
            &mut transpositions[bs..],
            parallelism,
            stack.rb_mut(),
        );

        for tmp in tmp_perm.iter_mut() {
            *tmp = perm[bs + *tmp];
        }
        perm[bs..].copy_from_slice(tmp_perm);
    }

    let parallelism = if m * (col_start + (full_n - (col_start + n))) > 128 * 128 {
        parallelism
    } else {
        Parallelism::None
    };
    if matrix.col_stride().abs() < matrix.row_stride().abs() {
        for (i, &t) in transpositions[..bs].iter().enumerate() {
            swap_rows(matrix.rb_mut().submatrix(0, 0, m, col_start), i, t + i);
        }
        for (i, &t) in transpositions[bs..].iter().enumerate() {
            swap_rows(
                matrix.rb_mut().submatrix(bs, 0, m - bs, col_start),
                i,
                t + i,
            );
        }
        for (i, &t) in transpositions[..bs].iter().enumerate() {
            swap_rows(
                matrix
                    .rb_mut()
                    .submatrix(0, col_start + n, m, full_n - col_start - n),
                i,
                t + i,
            );
        }
        for (i, &t) in transpositions[bs..].iter().enumerate() {
            swap_rows(
                matrix
                    .rb_mut()
                    .submatrix(bs, col_start + n, m - bs, full_n - col_start - n),
                i,
                t + i,
            );
        }
    } else if matrix.row_stride() == 1 {
        faer_core::for_each_raw(
            col_start + (full_n - (col_start + n)),
            |j| {
                let j = if j >= col_start { col_start + n + j } else { j };
                let mut col = unsafe { matrix.rb().col(j).const_cast() };
                for (i, &t) in transpositions[..bs].iter().enumerate() {
                    swap_two_elems_contiguous(col.rb_mut(), i, t + i);
                }
                let [_, mut col] = col.split_at_row(bs);
                for (i, &t) in transpositions[bs..].iter().enumerate() {
                    swap_two_elems_contiguous(col.rb_mut(), i, t + i);
                }
            },
            parallelism,
        );
    } else {
        faer_core::for_each_raw(
            col_start + (full_n - (col_start + n)),
            |j| {
                let j = if j >= col_start { col_start + n + j } else { j };
                let mut col = unsafe { matrix.rb().col(j).const_cast() };
                for (i, &t) in transpositions[..bs].iter().enumerate() {
                    swap_two_elems(col.rb_mut(), i, t + i);
                }
                let [_, mut col] = col.split_at_row(bs);
                for (i, &t) in transpositions[bs..].iter().enumerate() {
                    swap_two_elems(col.rb_mut(), i, t + i);
                }
            },
            parallelism,
        );
    }

    n_transpositions
}

#[derive(Default, Copy, Clone)]
#[non_exhaustive]
pub struct PartialPivLuComputeParams {}

/// Computes the size and alignment of required workspace for performing an LU
/// decomposition with partial pivoting.
pub fn lu_in_place_req<E: Entity>(
    m: usize,
    n: usize,
    parallelism: Parallelism,
    params: PartialPivLuComputeParams,
) -> Result<StackReq, SizeOverflow> {
    let _ = &params;

    let size = <usize as Ord>::min(n, m);
    StackReq::try_any_of([
        StackReq::try_new::<usize>(size)?,
        lu_recursive_req::<E>(m, size, parallelism)?,
    ])
}

/// Computes the LU decomposition of the given matrix with partial pivoting, replacing the matrix
/// with its factors in place.
///
/// The decomposition is such that:
/// $$PA = LU,$$
/// where $P$ is a permutation matrix, $L$ is a unit lower triangular matrix, and $U$ is an upper
/// triangular matrix.
///
/// $L$ is stored in the strictly lower triangular half of `matrix`, with an implicit unit
/// diagonal, $U$ is stored in the upper triangular half of `matrix`, and the permutation
/// representing $P$, as well as its inverse, are stored in `perm` and `perm_inv` respectively.
///
/// # Output
///
/// - The number of transpositions that constitute the permutation,
/// - a structure representing the permutation $P$.
///
/// # Panics
///
/// - Panics if the length of the permutation slices is not equal to the number of rows of the
/// matrix, or if the provided memory in `stack` is insufficient.
/// - Panics if the provided memory in `stack` is insufficient.
pub fn lu_in_place<'out, E: ComplexField>(
    matrix: MatMut<'_, E>,
    perm: &'out mut [usize],
    perm_inv: &'out mut [usize],
    parallelism: Parallelism,
    stack: DynStack<'_>,
    params: PartialPivLuComputeParams,
) -> (usize, PermutationMut<'out>) {
    let _ = &params;

    assert!(perm.len() == matrix.nrows());
    assert!(perm_inv.len() == matrix.nrows());
    let mut matrix = matrix;
    let mut stack = stack;
    let m = matrix.nrows();
    let n = matrix.ncols();
    let size = <usize as Ord>::min(n, m);

    for i in 0..m {
        perm[i] = i;
    }

    let n_transpositions = {
        let (mut transpositions, mut stack) = stack.rb_mut().make_with(size, |_| 0);

        lu_in_place_impl(
            matrix.rb_mut(),
            0,
            size,
            perm,
            &mut transpositions,
            parallelism,
            stack.rb_mut(),
        )
    };

    let [_, _, left, right] = matrix.split_at(0, size);

    if m < n {
        solve_unit_lower_triangular_in_place(left.rb(), right, parallelism);
    }

    for i in 0..m {
        perm_inv[perm[i]] = i;
    }

    (n_transpositions, unsafe {
        PermutationMut::new_unchecked(perm, perm_inv)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::partial_pivoting::reconstruct;
    use assert2::assert;
    use assert_approx_eq::assert_approx_eq;
    use dyn_stack::GlobalMemBuffer;
    use faer_core::{permutation::PermutationRef, Mat, MatRef};
    use rand::random;

    macro_rules! make_stack {
        ($req: expr) => {
            ::dyn_stack::DynStack::new(&mut ::dyn_stack::GlobalMemBuffer::new($req.unwrap()))
        };
    }

    fn reconstruct_matrix<E: ComplexField>(
        lu_factors: MatRef<'_, E>,
        row_perm: PermutationRef<'_>,
    ) -> Mat<E> {
        let m = lu_factors.nrows();
        let n = lu_factors.ncols();
        let mut dst = Mat::zeros(m, n);
        reconstruct::reconstruct(
            dst.as_mut(),
            lu_factors,
            row_perm,
            Parallelism::Rayon(0),
            make_stack!(reconstruct::reconstruct_req::<E>(
                m,
                n,
                Parallelism::Rayon(0)
            )),
        );
        dst
    }

    #[test]
    fn compute_lu() {
        for (m, n) in [
            (10, 10),
            (4, 4),
            (2, 4),
            (2, 20),
            (2, 2),
            (20, 20),
            (4, 2),
            (20, 2),
            (40, 20),
            (20, 40),
            (40, 60),
            (60, 40),
            (200, 100),
            (100, 200),
            (200, 200),
        ] {
            let mut mat = Mat::with_dims(m, n, |_, _| random::<f64>());
            let mat_orig = mat.clone();
            let mut perm = vec![0; m];
            let mut perm_inv = vec![0; m];

            let mut mem = GlobalMemBuffer::new(
                lu_in_place_req::<f64>(m, n, Parallelism::Rayon(8), Default::default()).unwrap(),
            );
            let mut stack = DynStack::new(&mut mem);

            let (_, row_perm) = lu_in_place(
                mat.as_mut(),
                &mut perm,
                &mut perm_inv,
                Parallelism::Rayon(8),
                stack.rb_mut(),
                Default::default(),
            );
            let reconstructed = reconstruct_matrix(mat.as_ref(), row_perm.rb());

            for i in 0..m {
                for j in 0..n {
                    assert_approx_eq!(mat_orig.read(i, j), reconstructed.read(i, j));
                }
            }
        }
    }

    #[test]
    fn compute_lu_non_contiguous() {
        for (m, n) in [
            (10, 10),
            (4, 4),
            (2, 4),
            (2, 20),
            (2, 2),
            (20, 20),
            (4, 2),
            (20, 2),
            (40, 20),
            (20, 40),
            (40, 60),
            (60, 40),
            (200, 100),
            (100, 200),
            (200, 200),
        ] {
            let mut mat = Mat::with_dims(m, n, |_, _| random::<f64>());
            let mut mat = mat.as_mut().reverse_rows();
            let mat_orig = mat.to_owned();
            let mut perm = vec![0; m];
            let mut perm_inv = vec![0; m];

            let mut mem = GlobalMemBuffer::new(
                lu_in_place_req::<f64>(m, n, Parallelism::Rayon(8), Default::default()).unwrap(),
            );
            let mut stack = DynStack::new(&mut mem);

            let (_, row_perm) = lu_in_place(
                mat.rb_mut(),
                &mut perm,
                &mut perm_inv,
                Parallelism::Rayon(8),
                stack.rb_mut(),
                Default::default(),
            );
            let reconstructed = reconstruct_matrix(mat.rb(), row_perm.rb());

            for i in 0..m {
                for j in 0..n {
                    assert_approx_eq!(mat_orig.read(i, j), reconstructed.read(i, j));
                }
            }
        }
    }

    #[test]
    fn compute_lu_row_major() {
        for (m, n) in [
            (10, 10),
            (4, 4),
            (2, 4),
            (2, 20),
            (2, 2),
            (20, 20),
            (4, 2),
            (20, 2),
            (40, 20),
            (20, 40),
            (40, 60),
            (60, 40),
            (200, 100),
            (100, 200),
            (200, 200),
        ] {
            let mut mat = Mat::with_dims(n, m, |_, _| random::<f64>());
            let mut mat = mat.as_mut().transpose();
            let mat_orig = mat.to_owned();
            let mut perm = vec![0; m];
            let mut perm_inv = vec![0; m];

            let mut mem = GlobalMemBuffer::new(
                lu_in_place_req::<f64>(m, n, Parallelism::Rayon(8), Default::default()).unwrap(),
            );
            let mut stack = DynStack::new(&mut mem);

            let (_, row_perm) = lu_in_place(
                mat.rb_mut(),
                &mut perm,
                &mut perm_inv,
                Parallelism::Rayon(8),
                stack.rb_mut(),
                Default::default(),
            );
            let reconstructed = reconstruct_matrix(mat.rb(), row_perm.rb());

            for i in 0..m {
                for j in 0..n {
                    assert_approx_eq!(mat_orig.read(i, j), reconstructed.read(i, j));
                }
            }
        }
    }
}
