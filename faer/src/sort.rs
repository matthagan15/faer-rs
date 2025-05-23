// code taken from the rust standard library

use crate::debug_assert;
use core::mem::MaybeUninit;

pub unsafe trait Ptr: Sized + Copy {
	type Item;

	fn get_ptr(ptr: *mut Self::Item) -> Self;

	fn null() -> Self;

	unsafe fn offset_from(self, origin: Self) -> isize;

	unsafe fn add(self, offset: usize) -> Self;
	unsafe fn sub(self, offset: usize) -> Self;

	unsafe fn read(self) -> Self::Item;
	unsafe fn write(self, item: Self::Item);

	unsafe fn copy_nonoverlapping(src: Self, dst: Self, len: usize);
	unsafe fn reverse(ptr: Self, len: usize);

	unsafe fn swap(a: Self, b: Self) {
		let a_item = a.read();
		let b_item = b.read();

		a.write(b_item);
		b.write(a_item);
	}

	unsafe fn swap_idx(self, i: usize, j: usize) {
		Self::swap(self.add(i), self.add(j));
	}
}

unsafe impl<T> Ptr for *mut T {
	type Item = MaybeUninit<T>;

	#[inline]
	fn get_ptr(ptr: *mut Self::Item) -> Self {
		ptr as *mut T
	}

	#[inline]
	fn null() -> Self {
		core::ptr::null_mut()
	}

	#[inline]
	unsafe fn offset_from(self, origin: Self) -> isize {
		self.offset_from(origin)
	}

	#[inline]
	unsafe fn add(self, offset: usize) -> Self {
		self.add(offset)
	}

	#[inline]
	unsafe fn sub(self, offset: usize) -> Self {
		self.sub(offset)
	}

	#[inline]
	unsafe fn read(self) -> Self::Item {
		(self as *mut MaybeUninit<T>).read()
	}

	#[inline]
	unsafe fn write(self, item: Self::Item) {
		(self as *mut MaybeUninit<T>).write(item)
	}

	#[inline]
	unsafe fn copy_nonoverlapping(src: Self, dst: Self, len: usize) {
		core::ptr::copy_nonoverlapping(src, dst, len);
	}

	#[inline]
	unsafe fn reverse(ptr: Self, len: usize) {
		core::slice::from_raw_parts_mut(ptr, len).reverse()
	}
}

unsafe impl<P: Ptr, Q: Ptr> Ptr for (P, Q) {
	type Item = (P::Item, Q::Item);

	#[inline]
	fn get_ptr(ptr: *mut Self::Item) -> Self {
		unsafe {
			(
				P::get_ptr(core::ptr::addr_of_mut!((*ptr).0)),
				Q::get_ptr(core::ptr::addr_of_mut!((*ptr).1)),
			)
		}
	}

	#[inline]
	fn null() -> Self {
		(P::null(), Q::null())
	}

	#[inline]
	unsafe fn offset_from(self, origin: Self) -> isize {
		self.0.offset_from(origin.0)
	}

	#[inline]
	unsafe fn add(self, offset: usize) -> Self {
		(self.0.add(offset), self.1.add(offset))
	}

	#[inline]
	unsafe fn sub(self, offset: usize) -> Self {
		(self.0.sub(offset), self.1.sub(offset))
	}

	#[inline]
	unsafe fn read(self) -> Self::Item {
		(self.0.read(), self.1.read())
	}

	#[inline]
	unsafe fn write(self, item: Self::Item) {
		self.0.write(item.0);
		self.1.write(item.1);
	}

	#[inline]
	unsafe fn copy_nonoverlapping(src: Self, dst: Self, len: usize) {
		P::copy_nonoverlapping(src.0, dst.0, len);
		Q::copy_nonoverlapping(src.1, dst.1, len);
	}

	#[inline]
	unsafe fn reverse(ptr: Self, len: usize) {
		P::reverse(ptr.0, len);
		Q::reverse(ptr.1, len);
	}
}

struct InsertionHole<P: Ptr> {
	src: P,
	dest: P,
}

impl<P: Ptr> Drop for InsertionHole<P> {
	#[inline(always)]
	fn drop(&mut self) {
		// SAFETY: This is a helper class. Please refer to its usage for correctness. Namely, one
		// must be sure that `src` and `dst` does not overlap as required by
		// `ptr::copy_nonoverlapping` and are both valid for writes.
		unsafe {
			P::copy_nonoverlapping(self.src, self.dest, 1);
		}
	}
}

unsafe fn insert_tail<P: Ptr, F>(v: P, v_len: usize, is_less: &mut F)
where
	F: FnMut(P, P) -> bool,
{
	debug_assert!(v_len >= 2);

	let arr_ptr = v;
	let i = v_len - 1;

	// SAFETY: caller must ensure v is at least len 2.
	unsafe {
		// See insert_head which talks about why this approach is beneficial.
		let i_ptr = arr_ptr.add(i);

		// It's important that we use i_ptr here. If this check is positive and we continue,
		// We want to make sure that no other copy of the value was seen by is_less.
		// Otherwise we would have to copy it back.
		if is_less(i_ptr, i_ptr.sub(1)) {
			// It's important, that we use tmp for comparison from now on. As it is the value that
			// will be copied back. And notionally we could have created a divergence if we copy
			// back the wrong value.
			let tmp = core::mem::ManuallyDrop::new(P::read(i_ptr));
			let tmp = P::get_ptr((&*tmp) as *const P::Item as *mut P::Item);
			// Intermediate state of the insertion process is always tracked by `hole`, which
			// serves two purposes:
			// 1. Protects integrity of `v` from panics in `is_less`.
			// 2. Fills the remaining hole in `v` in the end.
			//
			// Panic safety:
			//
			// If `is_less` panics at any point during the process, `hole` will get dropped and
			// fill the hole in `v` with `tmp`, thus ensuring that `v` still holds every object it
			// initially held exactly once.
			let mut hole = InsertionHole {
				src: tmp,
				dest: i_ptr.sub(1),
			};
			P::copy_nonoverlapping(hole.dest, i_ptr, 1);

			// SAFETY: We know i is at least 1.
			for j in (0..(i - 1)).rev() {
				let j_ptr = arr_ptr.add(j);
				if !is_less(tmp, j_ptr) {
					break;
				}

				P::copy_nonoverlapping(j_ptr, hole.dest, 1);
				hole.dest = j_ptr;
			}
			// `hole` gets dropped and thus copies `tmp` into the remaining hole in `v`.
		}
	}
}

unsafe fn insert_head<P: Ptr, F>(v: P, v_len: usize, is_less: &mut F)
where
	F: FnMut(P, P) -> bool,
{
	debug_assert!(v_len >= 2);

	// SAFETY: caller must ensure v is at least len 2.
	unsafe {
		if is_less(v.add(1), v.add(0)) {
			let arr_ptr = v;

			// There are three ways to implement insertion here:
			//
			// 1. Swap adjacent elements until the first one gets to its final destination. However, this way we
			//    copy data around more than is necessary. If elements are big structures (costly to copy), this
			//    method will be slow.
			//
			// 2. Iterate until the right place for the first element is found. Then shift the elements
			//    succeeding it to make room for it and finally place it into the remaining hole. This is a good
			//    method.
			//
			// 3. Copy the first element into a temporary variable. Iterate until the right place for it is
			//    found. As we go along, copy every traversed element into the slot preceding it. Finally, copy
			//    data from the temporary variable into the remaining hole. This method is very good. Benchmarks
			//    demonstrated slightly better performance than with the 2nd method.
			//
			// All methods were benchmarked, and the 3rd showed best results. So we chose that one.
			let tmp = core::mem::ManuallyDrop::new(P::read(arr_ptr));
			let tmp = P::get_ptr((&*tmp) as *const P::Item as *mut P::Item);

			// Intermediate state of the insertion process is always tracked by `hole`, which
			// serves two purposes:
			// 1. Protects integrity of `v` from panics in `is_less`.
			// 2. Fills the remaining hole in `v` in the end.
			//
			// Panic safety:
			//
			// If `is_less` panics at any point during the process, `hole` will get dropped and
			// fill the hole in `v` with `tmp`, thus ensuring that `v` still holds every object it
			// initially held exactly once.
			let mut hole = InsertionHole {
				src: tmp,
				dest: arr_ptr.add(1),
			};
			P::copy_nonoverlapping(arr_ptr.add(1), arr_ptr.add(0), 1);

			for i in 2..v_len {
				if !is_less(v.add(i), tmp) {
					break;
				}
				P::copy_nonoverlapping(arr_ptr.add(i), arr_ptr.add(i - 1), 1);
				hole.dest = arr_ptr.add(i);
			}
			// `hole` gets dropped and thus copies `tmp` into the remaining hole in `v`.
		}
	}
}

#[inline(never)]
pub(super) fn insertion_sort_shift_left<P: Ptr, F: FnMut(P, P) -> bool>(v: P, v_len: usize, offset: usize, is_less: &mut F) {
	let len = v_len;

	// Using assert here improves performance.
	core::assert!(offset != 0 && offset <= len);

	// Shift each element of the unsorted region v[i..] as far left as is needed to make v sorted.
	for i in offset..len {
		// SAFETY: we tested that `offset` must be at least 1, so this loop is only entered if len
		// >= 2. The range is exclusive and we know `i` must be at least 1 so this slice has at
		// >least len 2.
		unsafe {
			insert_tail(v, i + 1, is_less);
		}
	}
}

#[inline(never)]
fn insertion_sort_shift_right<P: Ptr, F: FnMut(P, P) -> bool>(v: P, v_len: usize, offset: usize, is_less: &mut F) {
	let len = v_len;

	// Using assert here improves performance.
	core::assert!(offset != 0 && offset <= len && len >= 2);

	// Shift each element of the unsorted region v[..i] as far left as is needed to make v sorted.
	for i in (0..offset).rev() {
		// SAFETY: we tested that `offset` must be at least 1, so this loop is only entered if len
		// >= 2.We ensured that the slice length is always at least 2 long. We know that start_found
		// will be at least one less than end, and the range is exclusive. Which gives us i always
		// <= (end - 2).
		unsafe {
			insert_head(v.add(i), len - i, is_less);
		}
	}
}

#[cold]
unsafe fn partial_insertion_sort<P: Ptr, F: FnMut(P, P) -> bool>(v: P, v_len: usize, is_less: &mut F) -> bool {
	// Maximum number of adjacent out-of-order pairs that will get shifted.
	const MAX_STEPS: usize = 5;
	// If the slice is shorter than this, don't shift any elements.
	const SHORTEST_SHIFTING: usize = 50;

	let len = v_len;
	let mut i = 1;

	for _ in 0..MAX_STEPS {
		// SAFETY: We already explicitly did the bound checking with `i < len`.
		// All our subsequent indexing is only in the range `0 <= index < len`
		unsafe {
			// Find the next pair of adjacent out-of-order elements.
			while i < len && !is_less(v.add(i), v.add(i - 1)) {
				i += 1;
			}
		}

		// Are we done?
		if i == len {
			return true;
		}

		// Don't shift elements on short arrays, that has a performance cost.
		if len < SHORTEST_SHIFTING {
			return false;
		}

		// Swap the found pair of elements. This puts them in correct order.
		v.swap_idx(i - 1, i);

		if i >= 2 {
			// Shift the smaller element to the left.
			insertion_sort_shift_left(v, i, i - 1, is_less);

			// Shift the greater element to the right.
			insertion_sort_shift_right(v, i, 1, is_less);
		}
	}

	// Didn't manage to sort the slice in the limited number of steps.
	false
}

#[cold]
pub unsafe fn heapsort<P: Ptr, F: FnMut(P, P) -> bool>(v: P, v_len: usize, mut is_less: F) {
	// This binary heap respects the invariant `parent >= child`.
	let mut sift_down = |v: P, v_len: usize, mut node| {
		loop {
			// Children of `node`.
			let mut child = 2 * node + 1;
			if child >= v_len {
				break;
			}

			// Choose the greater child.
			if child + 1 < v_len {
				// We need a branch to be sure not to out-of-bounds index,
				// but it's highly predictable.  The comparison, however,
				// is better done branchless, especially for primitives.
				child += is_less(v.add(child), v.add(child + 1)) as usize;
			}

			// Stop if the invariant holds at `node`.
			if !is_less(v.add(node), v.add(child)) {
				break;
			}

			// Swap `node` with the greater child, move one step down, and continue sifting.
			v.swap_idx(node, child);
			node = child;
		}
	};

	// Build the heap in linear time.
	for i in (0..v_len / 2).rev() {
		sift_down(v, v_len, i);
	}

	// Pop maximal elements from the heap.
	for i in (1..v_len).rev() {
		v.swap_idx(0, i);
		sift_down(v, i, 0);
	}
}

unsafe fn partition_in_blocks<P: Ptr, F: FnMut(P, P) -> bool>(v: P, v_len: usize, pivot: P, is_less: &mut F) -> usize {
	// Number of elements in a typical block.
	const BLOCK: usize = 128;

	// The partitioning algorithm repeats the following steps until completion:
	//
	// 1. Trace a block from the left side to identify elements greater than or equal to the pivot.
	// 2. Trace a block from the right side to identify elements smaller than the pivot.
	// 3. Exchange the identified elements between the left and right side.
	//
	// We keep the following variables for a block of elements:
	//
	// 1. `block` - Number of elements in the block.
	// 2. `start` - Start pointer into the `offsets` array.
	// 3. `end` - End pointer into the `offsets` array.
	// 4. `offsets` - Indices of out-of-order elements within the block.

	// The current block on the left side (from `l` to `l.add(block_l)`).
	let mut l = v;
	let mut block_l = BLOCK;
	let mut start_l = core::ptr::null_mut();
	let mut end_l = core::ptr::null_mut();
	let mut offsets_l = [core::mem::MaybeUninit::<u8>::uninit(); BLOCK];

	// The current block on the right side (from `r.sub(block_r)` to `r`).
	// SAFETY: The documentation for .add() specifically mention that `vec.as_ptr().add(vec.len())`
	// is always safe
	let mut r = unsafe { l.add(v_len) };
	let mut block_r = BLOCK;
	let mut start_r = core::ptr::null_mut();
	let mut end_r = core::ptr::null_mut();
	let mut offsets_r = [core::mem::MaybeUninit::<u8>::uninit(); BLOCK];

	// FIXME: When we get VLAs, try creating one array of length `min(v.len(), 2 * BLOCK)` rather
	// than two fixed-size arrays of length `BLOCK`. VLAs might be more cache-efficient.

	// Returns the number of elements between pointers `l` (inclusive) and `r` (exclusive).
	unsafe fn width<P: Ptr>(l: P, r: P) -> usize {
		r.offset_from(l) as usize
	}

	loop {
		// We are done with partitioning block-by-block when `l` and `r` get very close. Then we do
		// some patch-up work in order to partition the remaining elements in between.
		let is_done = width(l, r) <= 2 * BLOCK;

		if is_done {
			// Number of remaining elements (still not compared to the pivot).
			let mut rem = width(l, r);
			if start_l < end_l || start_r < end_r {
				rem -= BLOCK;
			}

			// Adjust block sizes so that the left and right block don't overlap, but get perfectly
			// aligned to cover the whole remaining gap.
			if start_l < end_l {
				block_r = rem;
			} else if start_r < end_r {
				block_l = rem;
			} else {
				// There were the same number of elements to switch on both blocks during the last
				// iteration, so there are no remaining elements on either block. Cover the
				// remaining items with roughly equally-sized blocks.
				block_l = rem / 2;
				block_r = rem - block_l;
			}
			debug_assert!(block_l <= BLOCK && block_r <= BLOCK);
			debug_assert!(width(l, r) == block_l + block_r);
		}

		if start_l == end_l {
			// Trace `block_l` elements from the left side.
			start_l = offsets_l.as_mut_ptr() as *mut u8;
			end_l = start_l;
			let mut elem = l;

			for i in 0..block_l {
				// SAFETY: The unsafety operations below involve the usage of the `offset`.
				//         According to the conditions required by the function, we satisfy them
				// because:
				//         1. `offsets_l` is stack-allocated, and thus considered separate allocated object.
				//         2. The function `is_less` returns a `bool`. Casting a `bool` will never overflow `isize`.
				//         3. We have guaranteed that `block_l` will be `<= BLOCK`. Plus, `end_l` was initially set
				//            to the begin pointer of `offsets_` which was declared on the stack. Thus, we know that
				//            even in the worst case (all invocations of `is_less` returns false) we will only be at
				//            most 1 byte pass the end.
				//        Another unsafety operation here is dereferencing `elem`.
				//        However, `elem` was initially the begin pointer to the slice which is
				// always valid.
				unsafe {
					// Branchless comparison.
					*end_l = i as u8;
					end_l = end_l.add(!is_less(elem, pivot) as usize);
					elem = elem.add(1);
				}
			}
		}

		if start_r == end_r {
			// Trace `block_r` elements from the right side.
			start_r = offsets_r.as_mut_ptr() as *mut u8;
			end_r = start_r;
			let mut elem = r;

			for i in 0..block_r {
				// SAFETY: The unsafety operations below involve the usage of the `offset`.
				//         According to the conditions required by the function, we satisfy them
				// because:
				//         1. `offsets_r` is stack-allocated, and thus considered separate allocated object.
				//         2. The function `is_less` returns a `bool`. Casting a `bool` will never overflow `isize`.
				//         3. We have guaranteed that `block_r` will be `<= BLOCK`. Plus, `end_r` was initially set
				//            to the begin pointer of `offsets_` which was declared on the stack. Thus, we know that
				//            even in the worst case (all invocations of `is_less` returns true) we will only be at
				//            most 1 byte pass the end.
				//        Another unsafety operation here is dereferencing `elem`.
				//        However, `elem` was initially `1 * sizeof(T)` past the end and we
				// decrement it by `1 * sizeof(T)` before accessing it.        Plus,
				// `block_r` was asserted to be less than `BLOCK` and `elem` will therefore at most
				// be pointing to the beginning of the slice.
				unsafe {
					// Branchless comparison.
					elem = elem.sub(1);
					*end_r = i as u8;
					end_r = end_r.add(is_less(elem, pivot) as usize);
				}
			}
		}

		// Number of out-of-order elements to swap between the left and right side.
		let count = Ord::min(width(start_l, end_l), width(start_r, end_r));

		if count > 0 {
			macro_rules! left {
				() => {
					l.add(usize::from(*start_l))
				};
			}
			macro_rules! right {
				() => {
					r.sub(usize::from(*start_r) + 1)
				};
			}

			// Instead of swapping one pair at the time, it is more efficient to perform a cyclic
			// permutation. This is not strictly equivalent to swapping, but produces a similar
			// result using fewer memory operations.

			// SAFETY: The use of `ptr::read` is valid because there is at least one element in
			// both `offsets_l` and `offsets_r`, so `left!` is a valid pointer to read from.
			//
			// The uses of `left!` involve calls to `offset` on `l`, which points to the
			// beginning of `v`. All the offsets pointed-to by `start_l` are at most `block_l`, so
			// these `offset` calls are safe as all reads are within the block. The same argument
			// applies for the uses of `right!`.
			//
			// The calls to `start_l.offset` are valid because there are at most `count-1` of them,
			// plus the final one at the end of the unsafe block, where `count` is the minimum
			// number of collected offsets in `offsets_l` and `offsets_r`, so there is
			// no risk of there not being enough elements. The same reasoning applies to
			// the calls to `start_r.offset`.
			//
			// The calls to `copy_nonoverlapping` are safe because `left!` and `right!` are
			// guaranteed not to overlap, and are valid because of the reasoning above.
			unsafe {
				let tmp = P::read(left!());
				let tmp_ptr = P::get_ptr(&tmp as *const P::Item as *mut P::Item);
				P::copy_nonoverlapping(right!(), left!(), 1);

				for _ in 1..count {
					start_l = start_l.add(1);
					P::copy_nonoverlapping(left!(), right!(), 1);
					start_r = start_r.add(1);
					P::copy_nonoverlapping(right!(), left!(), 1);
				}

				P::copy_nonoverlapping(tmp_ptr, right!(), 1);
				// core::mem::forget(tmp);
				start_l = start_l.add(1);
				start_r = start_r.add(1);
			}
		}

		if start_l == end_l {
			// All out-of-order elements in the left block were moved. Move to the next block.

			// block-width-guarantee
			// SAFETY: if `!is_done` then the slice width is guaranteed to be at least `2*BLOCK`
			// wide. There are at most `BLOCK` elements in `offsets_l` because of its
			// size, so the `offset` operation is safe. Otherwise, the debug assertions
			// in the `is_done` case guarantee that `width(l, r) == block_l + block_r`,
			// namely, that the block sizes have been adjusted to account
			// for the smaller number of remaining elements.
			l = unsafe { l.add(block_l) };
		}

		if start_r == end_r {
			// All out-of-order elements in the right block were moved. Move to the previous block.

			// SAFETY: Same argument as [block-width-guarantee]. Either this is a full block
			// `2*BLOCK`-wide, or `block_r` has been adjusted for the last handful of
			// elements.
			r = unsafe { r.sub(block_r) };
		}

		if is_done {
			break;
		}
	}

	// All that remains now is at most one block (either the left or the right) with out-of-order
	// elements that need to be moved. Such remaining elements can be simply shifted to the end
	// within their block.

	if start_l < end_l {
		// The left block remains.
		// Move its remaining out-of-order elements to the far right.
		debug_assert_eq!(width(l, r), block_l);
		while start_l < end_l {
			// remaining-elements-safety
			// SAFETY: while the loop condition holds there are still elements in `offsets_l`, so it
			// is safe to point `end_l` to the previous element.
			//
			// The `ptr::swap` is safe if both its arguments are valid for reads and writes:
			//  - Per the debug assert above, the distance between `l` and `r` is `block_l` elements, so there
			//    can be at most `block_l` remaining offsets between `start_l` and `end_l`. This means `r` will
			//    be moved at most `block_l` steps back, which makes the `r.offset` calls valid (at that point
			//    `l == r`).
			//  - `offsets_l` contains valid offsets into `v` collected during the partitioning of the last
			//    block, so the `l.offset` calls are valid.
			unsafe {
				end_l = end_l.sub(1);
				P::swap(l.add(usize::from(*end_l)), r.sub(1));
				r = r.sub(1);
			}
		}
		width(v, r)
	} else if start_r < end_r {
		// The right block remains.
		// Move its remaining out-of-order elements to the far left.
		debug_assert_eq!(width(l, r), block_r);
		while start_r < end_r {
			// SAFETY: See the reasoning in [remaining-elements-safety].
			unsafe {
				end_r = end_r.sub(1);
				P::swap(l, r.sub(usize::from(*end_r) + 1));
				l = l.add(1);
			}
		}
		width(v, l)
	} else {
		// Nothing else to do, we're done.
		width(v, l)
	}
}

pub(super) unsafe fn partition<P: Ptr, F>(v: P, v_len: usize, pivot: usize, is_less: &mut F) -> (usize, bool)
where
	F: FnMut(P, P) -> bool,
{
	let (mid, was_partitioned) = {
		// Place the pivot at the beginning of slice.
		v.swap_idx(0, pivot);
		let pivot = v;
		let v = v.add(1);
		let v_len = v_len - 1;

		// Read the pivot into a stack-allocated variable for efficiency. If a following comparison
		// operation panics, the pivot will be automatically written back into the slice.

		// SAFETY: `pivot` is a reference to the first element of `v`, so `ptr::read` is safe.
		let tmp = core::mem::ManuallyDrop::new(unsafe { P::read(pivot) });
		let tmp = P::get_ptr((&*tmp) as *const P::Item as *mut P::Item);
		let _pivot_guard = InsertionHole { src: tmp, dest: pivot };
		let pivot = tmp;

		// Find the first pair of out-of-order elements.
		let mut l = 0;
		let mut r = v_len;

		// SAFETY: The unsafety below involves indexing an array.
		// For the first one: We already do the bounds checking here with `l < r`.
		// For the second one: We initially have `l == 0` and `r == v.len()` and we checked that `l
		// < r` at every indexing operation.                     From here we know that `r`
		// must be at least `r == l` which was shown to be valid from the first one.
		unsafe {
			// Find the first element greater than or equal to the pivot.
			while l < r && is_less(v.add(l), pivot) {
				l += 1;
			}

			// Find the last element smaller that the pivot.
			while l < r && !is_less(v.add(r - 1), pivot) {
				r -= 1;
			}
		}

		(l + partition_in_blocks(v.add(l), r - l, pivot, is_less), l >= r)

		// `_pivot_guard` goes out of scope and writes the pivot (which is a stack-allocated
		// variable) back into the slice where it originally was. This step is critical in ensuring
		// safety!
	};

	// Place the pivot between the two partitions.
	v.swap_idx(0, mid);

	(mid, was_partitioned)
}

pub(super) unsafe fn partition_equal<P: Ptr, F>(v: P, v_len: usize, pivot: usize, is_less: &mut F) -> usize
where
	F: FnMut(P, P) -> bool,
{
	// Place the pivot at the beginning of slice.
	v.swap_idx(0, pivot);
	let pivot = v;
	let v = v.add(1);
	let v_len = v_len - 1;

	// Read the pivot into a stack-allocated variable for efficiency. If a following comparison
	// operation panics, the pivot will be automatically written back into the slice.
	// SAFETY: The pointer here is valid because it is obtained from a reference to a slice.
	let tmp = core::mem::ManuallyDrop::new(unsafe { P::read(pivot) });
	let tmp = P::get_ptr((&*tmp) as *const P::Item as *mut P::Item);
	let _pivot_guard = InsertionHole { src: tmp, dest: pivot };
	let pivot = tmp;

	let len = v_len;
	if len == 0 {
		return 0;
	}

	// Now partition the slice.
	let mut l = 0;
	let mut r = len;
	loop {
		// SAFETY: The unsafety below involves indexing an array.
		// For the first one: We already do the bounds checking here with `l < r`.
		// For the second one: We initially have `l == 0` and `r == v.len()` and we checked that `l
		// < r` at every indexing operation.                     From here we know that `r`
		// must be at least `r == l` which was shown to be valid from the first one.
		unsafe {
			// Find the first element greater than the pivot.
			while l < r && !is_less(pivot, v.add(l)) {
				l += 1;
			}

			// Find the last element equal to the pivot.
			loop {
				r -= 1;
				if l >= r || !is_less(pivot, v.add(r)) {
					break;
				}
			}

			// Are we done?
			if l >= r {
				break;
			}

			// Swap the found pair of out-of-order elements.
			let ptr = v;
			P::swap(ptr.add(l), ptr.add(r));
			l += 1;
		}
	}

	// We found `l` elements equal to the pivot. Add 1 to account for the pivot itself.
	l + 1

	// `_pivot_guard` goes out of scope and writes the pivot (which is a stack-allocated variable)
	// back into the slice where it originally was. This step is critical in ensuring safety!
}

#[cold]
pub(super) unsafe fn break_patterns<P: Ptr>(v: P, v_len: usize) {
	let len = v_len;
	if len >= 8 {
		let mut seed = len;
		let mut gen_usize = || {
			// Pseudorandom number generator from the "Xorshift RNGs" paper by George Marsaglia.
			if usize::BITS <= 32 {
				let mut r = seed as u32;
				r ^= r << 13;
				r ^= r >> 17;
				r ^= r << 5;
				seed = r as usize;
				seed
			} else {
				let mut r = seed as u64;
				r ^= r << 13;
				r ^= r >> 7;
				r ^= r << 17;
				seed = r as usize;
				seed
			}
		};

		// Take random numbers modulo this number.
		// The number fits into `usize` because `len` is not greater than `isize::MAX`.
		let modulus = len.next_power_of_two();

		// Some pivot candidates will be in the nearby of this index. Let's randomize them.
		let pos = len / 4 * 2;

		for i in 0..3 {
			// Generate a random number modulo `len`. However, in order to avoid costly operations
			// we first take it modulo a power of two, and then decrease by `len` until it fits
			// into the range `[0, len - 1]`.
			let mut other = gen_usize() & (modulus - 1);

			// `other` is guaranteed to be less than `2 * len`.
			if other >= len {
				other -= len;
			}

			v.swap_idx(pos - 1 + i, other);
		}
	}
}

pub(super) unsafe fn choose_pivot<P: Ptr, F>(v: P, v_len: usize, is_less: &mut F) -> (usize, bool)
where
	F: FnMut(P, P) -> bool,
{
	// Minimum length to choose the median-of-medians method.
	// Shorter slices use the simple median-of-three method.
	const SHORTEST_MEDIAN_OF_MEDIANS: usize = 50;
	// Maximum number of swaps that can be performed in this function.
	const MAX_SWAPS: usize = 4 * 3;

	let len = v_len;

	// Three indices near which we are going to choose a pivot.
	let mut a = len / 4;
	let mut b = len / 4 * 2;
	let mut c = len / 4 * 3;

	// Counts the total number of swaps we are about to perform while sorting indices.
	let mut swaps = 0;

	if len >= 8 {
		// Swaps indices so that `v[a] <= v[b]`.
		// SAFETY: `len >= 8` so there are at least two elements in the neighborhoods of
		// `a`, `b` and `c`. This means the three calls to `sort_adjacent` result in
		// corresponding calls to `sort3` with valid 3-item neighborhoods around each
		// pointer, which in turn means the calls to `sort2` are done with valid
		// references. Thus the `v.get_unchecked` calls are safe, as is the `ptr::swap`
		// call.
		let mut sort2 = |a: &mut usize, b: &mut usize| unsafe {
			if is_less(v.add(*b), v.add(*a)) {
				core::ptr::swap(a, b);
				swaps += 1;
			}
		};

		// Swaps indices so that `v[a] <= v[b] <= v[c]`.
		let mut sort3 = |a: &mut usize, b: &mut usize, c: &mut usize| {
			sort2(a, b);
			sort2(b, c);
			sort2(a, b);
		};

		if len >= SHORTEST_MEDIAN_OF_MEDIANS {
			// Finds the median of `v[a - 1], v[a], v[a + 1]` and stores the index into `a`.
			let mut sort_adjacent = |a: &mut usize| {
				let tmp = *a;
				sort3(&mut (tmp - 1), a, &mut (tmp + 1));
			};

			// Find medians in the neighborhoods of `a`, `b`, and `c`.
			sort_adjacent(&mut a);
			sort_adjacent(&mut b);
			sort_adjacent(&mut c);
		}

		// Find the median among `a`, `b`, and `c`.
		sort3(&mut a, &mut b, &mut c);
	}

	if swaps < MAX_SWAPS {
		(b, swaps == 0)
	} else {
		// The maximum number of swaps was performed. Chances are the slice is descending or mostly
		// descending, so reversing will probably help sort it faster.
		P::reverse(v, v_len);
		(len - 1 - b, true)
	}
}

/// sorts `v` using pattern-defeating quicksort, which is *O*(*n* \* log(*n*)) worst-case
pub unsafe fn quicksort<P: Ptr, F>(v: P, v_len: usize, mut is_less: F)
where
	F: FnMut(P, P) -> bool,
{
	// Sorting has no meaningful behavior on zero-sized types.
	if core::mem::size_of::<P::Item>() == 0 {
		return;
	}

	// Limit the number of imbalanced partitions to `floor(log2(len)) + 1`.
	let limit = usize::BITS - v_len.leading_zeros();

	recurse(v, v_len, &mut is_less, None, limit);
}

unsafe fn recurse<P: Ptr, F: FnMut(P, P) -> bool>(mut v: P, mut v_len: usize, is_less: &mut F, mut pred: Option<P>, mut limit: u32) {
	// Slices of up to this length get sorted using insertion sort.
	const MAX_INSERTION: usize = 20;

	// True if the last partitioning was reasonably balanced.
	let mut was_balanced = true;
	// True if the last partitioning didn't shuffle elements (the slice was already partitioned).
	let mut was_partitioned = true;

	loop {
		let len = v_len;

		// Very short slices get sorted using insertion sort.
		if len <= MAX_INSERTION {
			if len >= 2 {
				insertion_sort_shift_left(v, v_len, 1, is_less);
			}
			return;
		}

		// If too many bad pivot choices were made, simply fall back to heapsort in order to
		// guarantee `O(n * log(n))` worst-case.
		if limit == 0 {
			heapsort(v, v_len, is_less);
			return;
		}

		// If the last partitioning was imbalanced, try breaking patterns in the slice by shuffling
		// some elements around. Hopefully we'll choose a better pivot this time.
		if !was_balanced {
			break_patterns(v, v_len);
			limit -= 1;
		}

		// Choose a pivot and try guessing whether the slice is already sorted.
		let (pivot, likely_sorted) = choose_pivot(v, v_len, is_less);

		// If the last partitioning was decently balanced and didn't shuffle elements, and if pivot
		// selection predicts the slice is likely already sorted...
		if was_balanced && was_partitioned && likely_sorted {
			// Try identifying several out-of-order elements and shifting them to correct
			// positions. If the slice ends up being completely sorted, we're done.
			if partial_insertion_sort(v, v_len, is_less) {
				return;
			}
		}

		// If the chosen pivot is equal to the predecessor, then it's the smallest element in the
		// slice. Partition the slice into elements equal to and elements greater than the pivot.
		// This case is usually hit when the slice contains many duplicate elements.
		if let Some(p) = pred {
			if !is_less(p, v.add(pivot)) {
				let mid = partition_equal(v, v_len, pivot, is_less);

				// Continue sorting elements greater than the pivot.
				v = v.add(mid);
				v_len -= mid;
				continue;
			}
		}

		// Partition the slice.
		let (mid, was_p) = partition(v, v_len, pivot, is_less);
		was_balanced = Ord::min(mid, len - mid) >= len / 8;
		was_partitioned = was_p;

		// Split the slice into `left`, `pivot`, and `right`.
		let left = v;
		let left_len = mid;
		let right = v.add(mid);
		let right_len = v_len - mid;
		let pivot = right;
		let right = right.add(1);
		let right_len = right_len - 1;

		// Recurse into the shorter side only in order to minimize the total number of recursive
		// calls and consume less stack space. Then just continue with the longer side (this is
		// akin to tail recursion).
		if left_len < right_len {
			recurse(left, left_len, is_less, pred, limit);
			v = right;
			v_len = right_len;
			pred = Some(pivot);
		} else {
			recurse(right, right_len, is_less, Some(pivot), limit);
			v = left;
			v_len = left_len;
		}
	}
}

pub unsafe fn sort_unstable_by<P: Ptr>(ptr: P, len: usize, compare: impl FnMut(P, P) -> core::cmp::Ordering) {
	let mut compare = compare;
	quicksort(
		ptr,
		len,
		#[inline(always)]
		|a, b| compare(a, b) == core::cmp::Ordering::Less,
	);
}

pub unsafe fn sort_indices<I: crate::Index, T>(indices: &mut [I], values: &mut [T]) {
	let len = indices.len();
	debug_assert!(values.len() == len);

	sort_unstable_by((indices.as_mut_ptr(), values.as_mut_ptr()), len, |(i, _), (j, _)| (*i).cmp(&*j));
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::assert;
	use crate::internal_prelude::*;
	use rand::rngs::StdRng;
	use rand::{Rng, SeedableRng};

	#[test]
	fn test_quicksort() {
		let mut a = [3, 2, 2, 4, 1];
		let mut b = [1.0, 2.0, 3.0, 4.0, 5.0];

		let len = a.len();

		unsafe { quicksort((a.as_mut_ptr(), b.as_mut_ptr()), len, |p, q| *p.0 < *q.0) };

		assert!(a == [1, 2, 2, 3, 4]);
		assert!(b == [5.0, 2.0, 3.0, 1.0, 4.0]);
	}

	#[test]
	fn test_quicksort_big() {
		let rng = &mut StdRng::seed_from_u64(0);

		let a = &mut *(0..1000).map(|_| rng.gen::<u32>()).collect::<Vec<_>>();
		let b = &mut *(0..1000).map(|_| rng.gen::<f64>()).collect::<Vec<_>>();

		let a_orig = &*a.to_vec();
		let b_orig = &*b.to_vec();

		let mut perm = (0..1000).collect::<Vec<_>>();
		perm.sort_unstable_by_key(|&i| a[i]);

		let len = a.len();

		unsafe { quicksort((a.as_mut_ptr(), b.as_mut_ptr()), len, |p, q| *p.0 < *q.0) };

		for i in 0..1000 {
			assert!(a_orig[perm[i]] == a[i]);
			assert!(b_orig[perm[i]] == b[i]);
		}
	}
}
