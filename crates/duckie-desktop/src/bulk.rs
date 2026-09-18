use duckie_model::Row;

pub struct BulkEdit {
    pub text: String,
    pub original: Vec<Row>,
    pub error: String,
}
impl BulkEdit {
    pub fn new(rows: &[Row], delimiter: char) -> Self {
        Self {
            text: rows
                .iter()
                .filter(|r| !r.name.is_empty() || !r.value.is_empty())
                .map(|r| {
                    format!(
                        "{}{delimiter}{}{}",
                        r.name,
                        if delimiter == ':' { " " } else { "" },
                        r.value
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            original: rows.to_vec(),
            error: String::new(),
        }
    }
    pub fn parse(&self, delimiter: char) -> Result<Vec<Row>, String> {
        let mut used = vec![false; self.original.len()];
        let mut parsed = vec![];
        for (index, line) in self.text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let Some((name, value)) = line.split_once(delimiter) else {
                return Err(format!(
                    "Line {} needs '{delimiter}'. Text has been kept.",
                    index + 1
                ));
            };
            let (name, value) = if delimiter == ':' {
                (name.trim(), value.strip_prefix(' ').unwrap_or(value))
            } else {
                (name, value)
            };
            if name.is_empty() || name.contains('\r') || value.contains('\r') {
                return Err(format!(
                    "Line {} needs a name and a single-line value.",
                    index + 1
                ));
            }
            // Match the next occurrence of this name. Untouched
            // query rows retain encoded/literal provenance; edits deliberately restore templates.
            let found = self
                .original
                .iter()
                .enumerate()
                .position(|(i, r)| !used[i] && r.name == name);
            let mut row = Row::new(name, value);
            if let Some(i) = found {
                used[i] = true;
                let old = &self.original[i];
                row.enabled = old.enabled;
                row.extra = old.extra.clone();
                if old.value == value {
                    row.raw = old.raw.clone();
                    row.raw_is_literal = old.raw_is_literal;
                }
            }
            parsed.push(row);
        }
        Ok(parsed)
    }
}
pub fn apply(editor: &mut Option<BulkEdit>, rows: &mut Vec<Row>, delimiter: char) -> bool {
    let Some(bulk) = editor else {
        return true;
    };
    match bulk.parse(delimiter) {
        Ok(parsed) => {
            *rows = parsed;
            *editor = None;
            true
        }
        Err(error) => {
            bulk.error = error;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicates_flags_delimiters_and_literal_provenance_survive() {
        let mut first = Row::new("key", "{{secret.token}}");
        first.enabled = false;
        first.raw = Some("key=%7B%7Bsecret.token%7D%7D".into());
        first.raw_is_literal = true;
        let mut bulk = BulkEdit::new(&[first, Row::new("key", "second")], '=');
        bulk.text.push_str("\nnew=a=b");
        let rows = bulk.parse('=').unwrap();
        assert!(!rows[0].enabled);
        assert!(rows[0].raw_is_literal);
        assert!(rows[1].enabled);
        assert_eq!(rows[2].value, "a=b");
        bulk.text = "key=changed\nkey=second".into();
        let rows = bulk.parse('=').unwrap();
        assert!(!rows[0].enabled);
        assert!(!rows[0].raw_is_literal);
        assert!(rows[0].raw.is_none());
    }
    #[test]
    fn parse_failure_preserves_editor_and_rows() {
        let mut rows = vec![Row::new("Header", "https://local:80")];
        let original = rows.clone();
        let mut bulk = Some(BulkEdit::new(&rows, ':'));
        assert_eq!(
            bulk.as_ref().unwrap().parse(':').unwrap()[0].value,
            "https://local:80"
        );
        bulk.as_mut().unwrap().text = "valid: yes\nmalformed".into();
        assert!(!apply(&mut bulk, &mut rows, ':'));
        assert!(rows == original);
        assert!(bulk.unwrap().text.ends_with("malformed"));
    }
}
