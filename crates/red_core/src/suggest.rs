//! "Did you mean" for names and keys: the fuzzy matcher every validator shares. Dependency-free, so crates that cannot link the whole engine (the 2D game crate) include it.

/// Levenshtein-ish "did you mean": names containing the query or within edit distance 2.
pub fn suggest<'a>(query: &str, candidates: impl Iterator<Item = &'a str>) -> Vec<String> {
    fn dist(a: &str, b: &str) -> usize {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut cur = vec![i];
            for j in 1..=b.len() {
                let c = usize::from(a[i - 1] != b[j - 1]);
                cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c));
            }
            prev = cur;
        }
        prev[b.len()]
    }
    let q = query.to_lowercase();
    let mut out: Vec<(usize, String)> = candidates
        .filter_map(|c| {
            let cl = c.to_lowercase();
            let d = dist(&q, &cl);
            (cl.contains(&q) || q.contains(&cl) || d <= 2).then(|| (d, c.to_string()))
        })
        .collect();
    out.sort();
    out.into_iter().take(6).map(|(_, c)| c).collect()
}
