//! Strict cURL exchange. Commands are parsed as data and are never executed.
use crate::{Body, Part, RequestDefinition, Row};
use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shell {
    Posix,
    PowerShell,
}

pub fn import(input: &str) -> Result<RequestDefinition> {
    let words = split(input)?;
    if words
        .first()
        .is_none_or(|w| !w.eq_ignore_ascii_case("curl") && !w.eq_ignore_ascii_case("curl.exe"))
    {
        bail!("The command must start with curl");
    }
    let mut request = RequestDefinition {
        name: "Imported cURL request".into(),
        ..Default::default()
    };
    let mut method = None;
    let mut data = Vec::new();
    let mut form = Vec::new();
    let mut file_body = None;
    let mut at = 1;
    while at < words.len() {
        let word = &words[at];
        let mut value = |name: &str| -> Result<String> {
            at += 1;
            words
                .get(at)
                .cloned()
                .with_context(|| format!("{name} requires a value"))
        };
        match word.as_str() {
            "-X" | "--request" => method = Some(value(word)?.to_uppercase()),
            "-H" | "--header" => {
                let header = value(word)?;
                let (name, value) = header
                    .split_once(':')
                    .context("A header must contain ':'")?;
                request
                    .headers
                    .push(Row::new(name.trim(), value.trim_start()));
            }
            "-d" | "--data" | "--data-ascii" | "--data-binary" => {
                let item = value(word)?;
                if let Some(path) = item.strip_prefix('@') {
                    file_body = Some(path.to_owned());
                } else {
                    data.push(item);
                }
            }
            "--data-raw" => data.push(value(word)?),
            "--data-urlencode" => {
                let field = value(word)?;
                let (name, value) = field
                    .split_once('=')
                    .context("--data-urlencode requires name=value")?;
                form.push(Part {
                    enabled: true,
                    name: name.into(),
                    value: value.into(),
                    file: false,
                });
            }
            "--url" => request.set_address(&value(word)?),
            "-F" | "--form" => {
                let field = value(word)?;
                let (name, raw) = field
                    .split_once('=')
                    .context("A form field must contain '='")?;
                let (file, value) = raw
                    .strip_prefix('@')
                    .map_or((false, raw), |path| (true, path));
                form.push(Part {
                    enabled: true,
                    name: name.into(),
                    value: value.into(),
                    file,
                });
            }
            "-G" | "--get" => method = Some("GET".into()),
            "--compressed" | "-s" | "--silent" | "-S" | "--show-error" => {}
            value if value.starts_with('-') => bail!("Unsupported cURL option: {value}"),
            value if request.url.is_empty() => request.set_address(value),
            value => bail!("Unexpected cURL argument: {value}"),
        }
        at += 1;
    }
    if request.url.is_empty() {
        bail!("The cURL command has no URL");
    }
    if [!form.is_empty(), !data.is_empty(), file_body.is_some()]
        .into_iter()
        .filter(|v| *v)
        .count()
        > 1
    {
        bail!("Mixing file, form, and textual data is not supported");
    }
    if let Some(path) = file_body {
        let content_type = request
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("content-type"))
            .map(|h| h.value.clone())
            .unwrap_or_else(|| "application/octet-stream".into());
        request.body = Body::File { path, content_type };
    } else if !form.is_empty() {
        request.body = if words
            .iter()
            .any(|word| matches!(word.as_str(), "-F" | "--form"))
        {
            Body::Multipart { parts: form }
        } else {
            Body::Form {
                rows: form
                    .into_iter()
                    .map(|p| Row::new(p.name, p.value))
                    .collect(),
            }
        };
    } else if !data.is_empty() {
        let text = data.join("&");
        let form_encoded = request.headers.iter().any(|h| {
            h.name.eq_ignore_ascii_case("content-type")
                && h.value
                    .to_ascii_lowercase()
                    .starts_with("application/x-www-form-urlencoded")
        });
        request.body = if form_encoded {
            Body::Form {
                rows: url::form_urlencoded::parse(text.as_bytes())
                    .map(|(n, v)| Row::new(n, v))
                    .collect(),
            }
        } else if serde_json::from_str::<serde_json::Value>(&text).is_ok() {
            Body::Json { text }
        } else {
            Body::Text { text }
        };
    }
    request.method = method.unwrap_or_else(|| {
        if matches!(request.body, Body::None) {
            "GET"
        } else {
            "POST"
        }
        .into()
    });
    Ok(request)
}

pub fn export(request: &RequestDefinition, shell: Shell, credentials: bool) -> Result<String> {
    let quote = |value: &str| match shell {
        Shell::Posix => format!("'{}'", value.replace('\'', "'\"'\"'")),
        Shell::PowerShell => format!("'{}'", value.replace('\'', "''")),
    };
    let mut args = vec![match shell {
        Shell::Posix => "curl".into(),
        Shell::PowerShell => "curl.exe".into(),
    }];
    args.extend(["--request".into(), quote(&request.method)]);
    for header in request.headers.iter().filter(|h| h.enabled) {
        let sensitive = is_sensitive(&header.name);
        let value = if sensitive && !credentials {
            "<redacted>"
        } else {
            &header.value
        };
        args.extend([
            "--header".into(),
            quote(&format!("{}: {value}", header.name)),
        ]);
    }
    match &request.body {
        Body::None => {}
        Body::Json { text } | Body::Text { text } => {
            args.extend(["--data-raw".into(), quote(text)])
        }
        Body::Form { rows } => {
            for row in rows.iter().filter(|r| r.enabled) {
                args.extend([
                    "--data-urlencode".into(),
                    quote(&format!("{}={}", row.name, row.value)),
                ]);
            }
        }
        Body::Multipart { parts } => {
            for p in parts.iter().filter(|p| p.enabled) {
                args.extend([
                    "--form".into(),
                    quote(&format!(
                        "{}={}{}",
                        p.name,
                        if p.file { "@" } else { "" },
                        p.value
                    )),
                ]);
            }
        }
        Body::File { path, .. } => {
            args.extend(["--data-binary".into(), quote(&format!("@{path}"))])
        }
    }
    args.push(quote(&request.address()));
    Ok(match shell {
        Shell::Posix => args.join(" \\\n  "),
        Shell::PowerShell => args.join(" `\n  "),
    })
}

fn is_sensitive(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "x-api-key" | "api-key"
    )
}

fn split(input: &str) -> Result<Vec<String>> {
    let mut words = vec![];
    let mut word = String::new();
    let mut quote = None;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == q {
                if q == '\'' && chars.peek() == Some(&'\'') {
                    chars.next();
                    word.push('\'');
                } else {
                    quote = None;
                }
            } else if ch == '\\' && q == '"' {
                word.push(chars.next().context("Trailing escape")?);
            } else {
                word.push(ch);
            }
        } else {
            match ch {
                '\'' | '"' => quote = Some(ch),
                '`' | '\\' if chars.peek().is_some_and(|c| *c == '\n' || *c == '\r') => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                }
                c if c.is_whitespace() => {
                    if !word.is_empty() {
                        words.push(std::mem::take(&mut word));
                    }
                }
                _ => word.push(ch),
            }
        }
    }
    if quote.is_some() {
        bail!("Unclosed quote in cURL command");
    }
    if !word.is_empty() {
        words.push(word);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_duplicates_json_and_query() {
        let r=import("curl -X POST -H 'X-A: one' -H 'X-A: two' -d '{\"ok\":true}' 'https://e.test/p?a=1&a=2'").unwrap();
        assert_eq!(r.headers.len(), 2);
        assert_eq!(r.query.len(), 2);
        assert!(matches!(r.body, Body::Json { .. }));
    }
    #[test]
    fn rejects_unknown_options() {
        assert!(
            import("curl --location https://e.test")
                .err()
                .unwrap()
                .to_string()
                .contains("--location")
        );
    }
    #[test]
    fn exports_redacted_and_round_trips() {
        let mut r = RequestDefinition {
            url: "https://e.test/x".into(),
            method: "POST".into(),
            ..Default::default()
        };
        r.headers.push(Row::new("Authorization", "Bearer secret"));
        r.body = Body::Text {
            text: "it's fine".into(),
        };
        let text = export(&r, Shell::Posix, false).unwrap();
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("secret"));
        let parsed = import(&text).unwrap();
        assert_eq!(parsed.method, "POST");
    }
    #[test]
    fn imports_file_and_url_encoded_fields_without_reading_files() {
        let file = import("curl --data-binary '@payload.bin' https://e.test").unwrap();
        assert!(matches!(file.body,Body::File{ref path,..} if path=="payload.bin"));
        let form =
            import("curl --data-urlencode 'q=a & b' --data-urlencode q=c https://e.test").unwrap();
        assert!(matches!(form.body,Body::Form{ref rows} if rows.len()==2));
    }
}
