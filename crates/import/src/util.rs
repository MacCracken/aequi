/// Levenshtein edit distance using the two-row O(min(m,n)) space algorithm.
pub fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let a = s1.as_bytes();
    let b = s2.as_bytes();
    let (m, n) = (a.len(), b.len());

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    // Keep the shorter string in the inner loop to minimise allocation.
    let (a, b, m, n) = if m <= n { (a, b, m, n) } else { (b, a, n, m) };

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_strings_are_zero() {
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn empty_string_is_length_of_other() {
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
    }

    #[test]
    fn single_substitution() {
        assert_eq!(levenshtein_distance("cat", "bat"), 1);
    }

    #[test]
    fn single_insertion() {
        assert_eq!(levenshtein_distance("abc", "abcd"), 1);
    }

    #[test]
    fn single_deletion() {
        assert_eq!(levenshtein_distance("abcd", "abc"), 1);
    }

    #[test]
    fn commutative() {
        assert_eq!(
            levenshtein_distance("amazon", "amzn"),
            levenshtein_distance("amzn", "amazon")
        );
    }
}
