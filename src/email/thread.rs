//! JWZ-lite threading (spec §5) — pure functions plus the ingest-time
//! thread_id computation (the only part that touches the DB).

/// Strip `Re:`/`Fwd:`/`Fw:` prefixes (looping), collapse whitespace,
/// lowercase. Empty/broken subjects normalize to "".
pub fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim().to_string();
    loop {
        let lower = s.to_lowercase();
        let stripped = ["re:", "fwd:", "fw:"].iter().find_map(|p| {
            lower
                .strip_prefix(p)
                .map(|rest| rest.trim_start().to_string())
        });
        match stripped {
            Some(next) => s = next,
            None => break,
        }
    }
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Threading candidates in priority order: In-Reply-To, last References
/// entry, else own Message-ID.
pub fn thread_candidates(
    message_id: Option<&str>,
    in_reply_to: Option<&str>,
    references: &[String],
) -> Vec<String> {
    let clean = |s: &str| s.trim().to_string();
    let mut out = Vec::new();
    if let Some(irt) = in_reply_to {
        let v = clean(irt);
        if !v.is_empty() {
            out.push(v);
        }
    }
    if let Some(last) = references.last() {
        let v = clean(last);
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    if out.is_empty() {
        if let Some(mid) = message_id {
            let v = clean(mid);
            if !v.is_empty() {
                out.push(v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_normalization() {
        assert_eq!(normalize_subject("Re: Re: FW: Hello There"), "hello there");
        assert_eq!(
            normalize_subject("fwd: Fw:  project  update "),
            "project update"
        );
        assert_eq!(normalize_subject("Re:RE:no-space"), "no-space");
        assert_eq!(normalize_subject(""), "");
        assert_eq!(normalize_subject("Re:"), "");
    }

    #[test]
    fn candidates_priority() {
        assert_eq!(
            thread_candidates(Some("<m3>"), Some("<m2>"), &["<m0>".into(), "<m1>".into()]),
            vec!["<m2>".to_string(), "<m1>".to_string()]
        );
        // No irt/references → own message-id.
        assert_eq!(
            thread_candidates(Some("<m3>"), None, &[]),
            vec!["<m3>".to_string()]
        );
        // irt duplicates last reference → deduped.
        assert_eq!(
            thread_candidates(Some("<m3>"), Some("<m1>"), &["<m1>".into()]),
            vec!["<m1>".to_string()]
        );
        // Nothing at all → empty.
        assert_eq!(thread_candidates(None, None, &[]), Vec::<String>::new());
    }
}
