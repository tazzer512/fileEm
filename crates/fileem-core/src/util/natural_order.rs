//! Case-insensitive ordering that compares digit runs numerically, so "file2" sorts before "file10".

use std::cmp::Ordering;

/// Compares `a` and `b` naturally. Only identical strings compare equal: names that differ only in
/// case or leading zeros are ordered by their text, so sorting is deterministic.
pub fn compare(a: &str, b: &str) -> Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < ab.len() && j < bb.len() {
        if ab[i].is_ascii_digit() && bb[j].is_ascii_digit() {
            let (mut si, mut sj) = (i, j);
            while si < ab.len() && ab[si] == b'0' {
                si += 1;
            }
            while sj < bb.len() && bb[sj] == b'0' {
                sj += 1;
            }
            let (mut ei, mut ej) = (si, sj);
            while ei < ab.len() && ab[ei].is_ascii_digit() {
                ei += 1;
            }
            while ej < bb.len() && bb[ej].is_ascii_digit() {
                ej += 1;
            }
            // more significant digits is the bigger number; same length compares digit by digit
            let by_value = (ei - si).cmp(&(ej - sj)).then_with(|| ab[si..ei].cmp(&bb[sj..ej]));
            if by_value != Ordering::Equal {
                return by_value;
            }
            i = ei;
            j = ej;
        } else {
            let ca = a[i..].chars().next().unwrap_or_default();
            let cb = b[j..].chars().next().unwrap_or_default();
            let d = fold(ca).cmp(&fold(cb));
            if d != Ordering::Equal {
                return d;
            }
            i += ca.len_utf8();
            j += cb.len_utf8();
        }
    }
    let rest = a[i..].chars().count().cmp(&b[j..].chars().count());
    rest.then_with(|| a.cmp(b))
}

/// Case folding as Java's `toLowerCase(toUpperCase(c))`, keeping characters whose case mapping
/// isn't a single character.
fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let upper = single(c.to_uppercase()).unwrap_or(c);
    single(upper.to_lowercase()).unwrap_or(upper)
}

fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let first = it.next()?;
    if it.next().is_some() {
        None
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_digit_runs_numerically() {
        let mut names = vec!["file10.txt", "file2.txt", "file1.txt", "file02.txt", "file100"];
        names.sort_by(|a, b| compare(a, b));
        // equal numbers with different leading zeros are ordered deterministically by their text
        assert_eq!(names, ["file1.txt", "file02.txt", "file2.txt", "file10.txt", "file100"]);
    }

    #[test]
    fn ignores_case() {
        assert!(compare("apple", "Banana").is_lt());
        assert!(compare("Apple", "banana").is_lt());
        assert!(compare("ABC", "abd").is_lt());
    }

    #[test]
    fn shorter_prefix_sorts_first_and_equal_is_zero() {
        assert!(compare("abc", "abcd").is_lt());
        assert_eq!(compare("same", "same"), Ordering::Equal);
        assert!(compare("", "a").is_lt());
    }

    #[test]
    fn handles_huge_numbers_without_overflow() {
        assert!(compare("x99999999999999999999", "x100000000000000000000").is_lt());
    }

    #[test]
    fn non_ascii_letters_fold_case() {
        assert!(compare("élan", "Zebra").is_gt(), "é sorts after z, as in Java");
        assert!(compare("Äpfel", "äpfel").is_lt(), "case-only differences are ordered by text");
        assert_ne!(compare("Äpfel", "äpfel"), Ordering::Equal);
    }

    #[test]
    fn is_a_consistent_comparator() {
        // a small LCG instead of a random crate: deterministic samples like the Java test's seed
        let mut seed: u64 = 42;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let alphabet: Vec<char> = "aAbB019 ._-é".chars().collect();
        let mut samples: Vec<String> = (0..300)
            .map(|_| (0..next(8)).map(|_| alphabet[next(alphabet.len() as u64) as usize]).collect())
            .collect();
        for a in &samples {
            for b in &samples {
                assert_eq!(compare(a, b), compare(b, a).reverse(), "{a:?} vs {b:?}");
                if compare(a, b) == Ordering::Equal {
                    assert_eq!(a, b, "only identical strings compare equal");
                }
            }
        }
        samples.sort_by(|a, b| compare(a, b)); // panics on a total order violation
        for w in samples.windows(2) {
            assert!(compare(&w[0], &w[1]).is_le());
        }
    }
}
