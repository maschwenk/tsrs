//! Go standard library `slices` functions ported with Go's exact algorithm (go1.27 `slices/sort.go`,
//! `slices/zsortanyfunc.go`), for comparators whose sequence of calls is observable: the checker's
//! `CompareTypes` / `compareSymbols` assign symbol ids lazily as a last resort, so which pairs a sort or
//! search compares decides which symbols get ids first. Rust's `sort_by` / `binary_search_by` return the
//! same results for a total order but compare different pairs.

/// Go `slices.BinarySearchFunc`.
#[inline]
pub fn binary_search_func<E, T: ?Sized>(x: &[E], target: &T, mut cmp: impl FnMut(&E, &T) -> i32) -> (usize, bool) {
    let n = x.len();
    let (mut i, mut j) = (0usize, n);
    while i < j {
        let h = (i + j) >> 1;
        if cmp(&x[h], target) < 0 {
            i = h + 1;
        } else {
            j = h;
        }
    }
    (i, i < n && cmp(&x[i], target) == 0)
}

/// Go `slices.SortStableFunc`.
pub fn sort_stable_func<E>(data: &mut [E], mut cmp: impl FnMut(&E, &E) -> i32) {
    let n = data.len();
    stable_cmp_func(data, n, &mut cmp);
}

fn insertion_sort_cmp_func<E>(data: &mut [E], a: usize, b: usize, cmp: &mut impl FnMut(&E, &E) -> i32) {
    for i in a + 1..b {
        let mut j = i;
        while j > a && cmp(&data[j], &data[j - 1]) < 0 {
            data.swap(j, j - 1);
            j -= 1;
        }
    }
}

fn swap_range_cmp_func<E>(data: &mut [E], a: usize, b: usize, n: usize) {
    for i in 0..n {
        data.swap(a + i, b + i);
    }
}

fn stable_cmp_func<E>(data: &mut [E], n: usize, cmp: &mut impl FnMut(&E, &E) -> i32) {
    let mut block_size = 20;
    let (mut a, mut b) = (0, block_size);
    while b <= n {
        insertion_sort_cmp_func(data, a, b, cmp);
        a = b;
        b += block_size;
    }
    insertion_sort_cmp_func(data, a, n, cmp);
    while block_size < n {
        a = 0;
        b = 2 * block_size;
        while b <= n {
            sym_merge_cmp_func(data, a, a + block_size, b, cmp);
            a = b;
            b += 2 * block_size;
        }
        let m = a + block_size;
        if m < n {
            sym_merge_cmp_func(data, a, m, n, cmp);
        }
        block_size *= 2;
    }
}

fn sym_merge_cmp_func<E>(data: &mut [E], a: usize, m: usize, b: usize, cmp: &mut impl FnMut(&E, &E) -> i32) {
    if m - a == 1 {
        let (mut i, mut j) = (m, b);
        while i < j {
            let h = (i + j) >> 1;
            if cmp(&data[h], &data[a]) < 0 {
                i = h + 1;
            } else {
                j = h;
            }
        }
        let mut k = a;
        while k + 1 < i {
            data.swap(k, k + 1);
            k += 1;
        }
        return;
    }
    if b - m == 1 {
        let (mut i, mut j) = (a, m);
        while i < j {
            let h = (i + j) >> 1;
            if !(cmp(&data[m], &data[h]) < 0) {
                i = h + 1;
            } else {
                j = h;
            }
        }
        let mut k = m;
        while k > i {
            data.swap(k, k - 1);
            k -= 1;
        }
        return;
    }
    let mid = (a + b) >> 1;
    let n = mid + m;
    let (mut start, mut r) = if m > mid { (n - b, mid) } else { (a, m) };
    let p = n - 1;
    while start < r {
        let c = (start + r) >> 1;
        if !(cmp(&data[p - c], &data[c]) < 0) {
            start = c + 1;
        } else {
            r = c;
        }
    }
    let end = n - start;
    if start < m && m < end {
        rotate_cmp_func(data, start, m, end);
    }
    if a < start && start < mid {
        sym_merge_cmp_func(data, a, start, mid, cmp);
    }
    if mid < end && end < b {
        sym_merge_cmp_func(data, mid, end, b, cmp);
    }
}

fn rotate_cmp_func<E>(data: &mut [E], a: usize, m: usize, b: usize) {
    let mut i = m - a;
    let mut j = b - m;
    while i != j {
        if i > j {
            swap_range_cmp_func(data, m - i, m, j);
            i -= j;
        } else {
            swap_range_cmp_func(data, m - i, m + j - i, i);
            j -= i;
        }
    }
    swap_range_cmp_func(data, m - i, m, i);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_sort_matches_std_and_keeps_equal_order() {
        let mut seed = 12345u64;
        for n in [0usize, 1, 2, 19, 20, 21, 39, 40, 41, 100, 257, 1000] {
            let mut v: Vec<(u32, usize)> = (0..n)
                .map(|i| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    ((seed >> 59) as u32, i)
                })
                .collect();
            let mut expected = v.clone();
            expected.sort_by_key(|e| e.0);
            sort_stable_func(&mut v, |a, b| a.0 as i32 - b.0 as i32);
            assert_eq!(v, expected, "n = {n}");
        }
    }

    #[test]
    fn binary_search_finds_insertion_point() {
        let v = [1, 3, 5, 7];
        assert_eq!(binary_search_func(&v, &5, |a, b| a - b), (2, true));
        assert_eq!(binary_search_func(&v, &4, |a, b| a - b), (2, false));
        assert_eq!(binary_search_func(&v, &9, |a, b| a - b), (4, false));
        assert_eq!(binary_search_func(&[] as &[i32], &1, |a, b| a - b), (0, false));
    }
}
