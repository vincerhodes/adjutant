//! Compose: RFC822 message building, reply quoting (plain text), and the
//! deliberately dumb HTML→text strip used when a message has no plain part.
//!
//! No HTML compose in M2 — outgoing mail is plain text only.

use crate::email::model::{Email, EmailAccount, MailAddress, OutboxItem};

/// Build an RFC822 message (CRLF line endings) for an outbox item.
pub fn build_rfc822(item: &OutboxItem, account: &EmailAccount) -> Vec<u8> {
    let mut out = String::new();
    let push = |out: &mut String, line: &str| {
        out.push_str(&line.replace('\n', "\r\n"));
        out.push_str("\r\n");
    };
    push(
        &mut out,
        &format!(
            "From: {}",
            display_addr(&account.address, Some(&account.label))
        ),
    );
    push(&mut out, &format!("To: {}", addr_list(&item.to)));
    if !item.cc.is_empty() {
        push(&mut out, &format!("Cc: {}", addr_list(&item.cc)));
    }
    push(&mut out, &format!("Subject: {}", item.subject));
    push(
        &mut out,
        &format!(
            "Date: {}",
            chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S +0000")
        ),
    );
    push(
        &mut out,
        &format!("Message-ID: <{}@adjutant.local>", item.id),
    );
    if let Some(irt) = &item.in_reply_to {
        push(&mut out, &format!("In-Reply-To: {irt}"));
    }
    push(&mut out, "MIME-Version: 1.0");
    push(&mut out, "Content-Type: text/plain; charset=utf-8");
    push(&mut out, "");
    out.push_str(&item.body.replace('\n', "\r\n"));
    out.into_bytes()
}

fn display_addr(addr: &str, name: Option<&str>) -> String {
    match name.filter(|n| !n.is_empty()) {
        Some(n) => format!("{n} <{addr}>"),
        None => addr.to_string(),
    }
}

fn addr_list(list: &[MailAddress]) -> String {
    list.iter()
        .map(|a| display_addr(&a.addr, a.name.as_deref()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `On <date>, <name> wrote:` + `> ` quoted lines (plain-text reply prefill).
pub fn quote_reply(source: &Email) -> String {
    let who = source
        .from_name
        .clone()
        .unwrap_or_else(|| source.from_addr.clone());
    let date = source.date.split('T').next().unwrap_or(&source.date);
    let mut out = format!("On {date}, {who} wrote:\n");
    let body = source.body_text.clone().unwrap_or_default();
    for line in body.lines() {
        out.push_str("> ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Deliberately dumb HTML → text (spec §4: no html2text dep; simple tag
/// strip + minimal entity decode). Quality is limited by design; raw HTML
/// is preserved in body_html for a future subset renderer.
pub fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    // Drop script/style blocks entirely.
    for tag in ["script", "style"] {
        loop {
            let lower = s.to_lowercase();
            let Some(start) = lower.find(&format!("<{tag}")) else {
                break;
            };
            let end_tag = format!("</{tag}>");
            let end = lower[start..]
                .find(&end_tag)
                .map(|e| start + e + end_tag.len())
                .unwrap_or(start);
            s.replace_range(start..end, " ");
        }
    }
    // Block-level tags become line breaks before the tag strip.
    for tag in [
        "<br", "<p", "<div", "<li", "<tr", "<h1", "<h2", "<h3", "<h4",
    ] {
        s = s.replace(tag, &format!("\n{tag}"));
    }
    // Strip all remaining tags.
    let mut stripped = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => stripped.push(c),
            _ => {}
        }
    }
    decode_entities(&stripped)
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_strip_basic() {
        let html = "<html><head><style>body{color:red}</style></head>\
                    <body><h1>Title</h1><p>Hello <b>world</b> &amp; friends</p>\
                    <script>evil()</script><br>next</body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world & friends"));
        assert!(!text.contains("evil"));
        assert!(text.contains("next"));
    }

    #[test]
    fn quote_reply_format() {
        let src = Email {
            from_name: Some("Sam".to_string()),
            from_addr: "sam@x.com".to_string(),
            date: "2026-10-01T09:00:00+00:00".to_string(),
            body_text: Some("line one\nline two".to_string()),
            ..fake_email()
        };
        let q = quote_reply(&src);
        assert!(q.starts_with("On 2026-10-01, Sam wrote:"));
        assert!(q.contains("> line one"));
        assert!(q.contains("> line two"));
    }

    #[test]
    fn rfc822_shape() {
        let account = EmailAccount {
            id: uuid::Uuid::nil(),
            label: "Jim".to_string(),
            address: "jim@x.com".to_string(),
            imap_host: String::new(),
            imap_port: 993,
            smtp_host: String::new(),
            smtp_port: 465,
            username: String::new(),
            color_idx: 0,
            sync_interval_s: 300,
            last_sync_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let item = OutboxItem {
            id: uuid::Uuid::nil(),
            account_id: account.id,
            state: crate::email::model::OutboxState::Staged,
            to: vec![MailAddress {
                name: Some("Sam".to_string()),
                addr: "sam@x.com".to_string(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Hi there".to_string(),
            body: "Body text".to_string(),
            in_reply_to: Some("<abc@x>".to_string()),
            source_email_id: None,
            origin: "ui".to_string(),
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
            sent_at: None,
        };
        let raw = build_rfc822(&item, &account);
        let text = String::from_utf8(raw).expect("rfc822 is utf-8");
        assert!(text.contains("From: Jim <jim@x.com>"));
        assert!(text.contains("To: Sam <sam@x.com>"));
        assert!(text.contains("Subject: Hi there"));
        assert!(text.contains("In-Reply-To: <abc@x>"));
        assert!(text.contains("Body text"));
        assert!(text.contains("\r\n"));
    }

    fn fake_email() -> Email {
        Email {
            id: uuid::Uuid::nil(),
            account_id: uuid::Uuid::nil(),
            folder_id: uuid::Uuid::nil(),
            uid: 0,
            message_id: None,
            thread_id: uuid::Uuid::nil(),
            from_name: None,
            from_addr: String::new(),
            to: vec![],
            cc: vec![],
            subject: String::new(),
            snippet: String::new(),
            date: String::new(),
            flags: vec![],
            has_attachments: false,
            body_text: None,
            body_html: None,
            body_fetched_at: None,
            size: 0,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
