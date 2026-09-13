//! OpenAPI JSON -> reviewable request drafts. Import never writes or executes requests.
use anyhow::{Context, Result, bail};
use duckie_model::*;
use serde_json::{Value, json};

pub struct ImportDraft {
    pub name: String,
    pub version: String,
    pub servers: Vec<String>,
    pub operations: Vec<Operation>,
    pub diagnostics: Vec<String>,
}
pub struct Operation {
    pub selected: bool,
    pub request: RequestDefinition,
    pub diagnostics: Vec<String>,
    pub bodies: Vec<(String, Body)>,
    pub body_index: usize,
    pub auth_options: Vec<(String, Auth, Vec<String>)>,
    pub auth_index: usize,
}

fn resolve<'a>(root: &'a Value, value: &'a Value, chain: &mut Vec<String>) -> Result<&'a Value> {
    if chain.len() >= 32 {
        bail!("Reference depth exceeds 32");
    }
    if let Some(reference) = value["$ref"].as_str() {
        if !reference.starts_with('#') {
            bail!("External reference needs resolution before Send: {reference}");
        }
        if chain.iter().any(|r| r == reference) {
            bail!("Reference cycle: {} -> {reference}", chain.join(" -> "));
        }
        chain.push(reference.into());
        let target = root
            .pointer(&reference[1..])
            .with_context(|| format!("Unresolved reference {reference}"))?;
        let result = resolve(root, target, chain);
        chain.pop();
        result
    } else {
        Ok(value)
    }
}
fn deref<'a>(root: &'a Value, value: &'a Value) -> Result<&'a Value> {
    resolve(root, value, &mut vec![])
}
fn sample(root: &Value, schema: &Value, depth: usize) -> Result<Value> {
    if depth > 16 {
        bail!("Example recursion exceeds 16; supply body data manually");
    }
    let schema = deref(root, schema)?;
    for key in ["example", "default", "const"] {
        if let Some(v) = schema.get(key) {
            return Ok(v.clone());
        }
    }
    for key in ["examples", "enum"] {
        if let Some(v) = schema[key].as_array().and_then(|a| a.first()) {
            return Ok(v.clone());
        }
    }
    // `allOf` is a conjunction, so every branch contributes its properties to one object.
    if let Some(branches) = schema["allOf"].as_array() {
        let mut merged = serde_json::Map::new();
        for branch in branches {
            match sample(root, branch, depth + 1)? {
                Value::Object(map) => merged.extend(map),
                _ => {
                    bail!("allOf combines schemas that are not objects; supply body data manually")
                }
            }
        }
        // Properties declared beside the allOf apply as well.
        merged.extend(object_properties(root, schema, depth)?);
        return Ok(Value::Object(merged));
    }
    // `oneOf`/`anyOf` are choices. Nested ones collapse to the first branch; a choice at the top
    // of a request body is offered to the user instead, in `variants`.
    for key in ["oneOf", "anyOf"] {
        if let Some(first) = schema[key].as_array().and_then(|list| list.first()) {
            return sample(root, first, depth + 1);
        }
    }
    let typ = schema["type"]
        .as_str()
        .or_else(|| {
            schema["type"]
                .as_array()
                .and_then(|a| a.iter().filter_map(Value::as_str).find(|t| *t != "null"))
        })
        .unwrap_or(if schema.get("properties").is_some() {
            "object"
        } else {
            "string"
        });
    Ok(match typ {
        "object" => Value::Object(object_properties(root, schema, depth)?),
        "array" => json!([sample(root, &schema["items"], depth + 1)?]),
        "integer" | "number" => json!(0),
        "boolean" => json!(false),
        "null" => Value::Null,
        _ => json!(match schema["format"].as_str() {
            Some("date") => "2026-01-01",
            Some("date-time") => "2026-01-01T00:00:00Z",
            Some("uuid") => "00000000-0000-0000-0000-000000000000",
            _ => "example",
        }),
    })
}
/// Sample values for a schema's own `properties`, omitting read-only ones, which a request body
/// must not carry.
fn object_properties(
    root: &Value,
    schema: &Value,
    depth: usize,
) -> Result<serde_json::Map<String, Value>> {
    let mut object = serde_json::Map::new();
    if let Some(properties) = schema["properties"].as_object() {
        for (name, property) in properties {
            let property = deref(root, property)?;
            if property["readOnly"] != true {
                object.insert(name.clone(), sample(root, property, depth + 1)?);
            }
        }
    }
    Ok(object)
}
/// The alternatives a request-body schema offers. A `oneOf`/`anyOf` at the top level becomes one
/// labelled candidate per branch, so the import review can pick rather than block.
fn variants(root: &Value, schema: &Value) -> Result<Vec<(String, Value)>> {
    let resolved = deref(root, schema)?;
    for key in ["oneOf", "anyOf"] {
        if let Some(branches) = resolved[key].as_array().filter(|list| !list.is_empty()) {
            return branches
                .iter()
                .enumerate()
                .map(|(i, branch)| {
                    let label = deref(root, branch)
                        .ok()
                        .and_then(|b| b["title"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| format!("{key} option {}", i + 1));
                    Ok((label, sample(root, branch, 1)?))
                })
                .collect();
        }
    }
    Ok(vec![(String::new(), sample(root, schema, 0)?)])
}
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => Some(String::new()),
        _ => None,
    }
}
/// An example array is usable only when every member serializes as a scalar.
fn list(value: &Value) -> Option<Vec<String>> {
    value.as_array()?.iter().map(scalar).collect()
}
fn server_urls(servers: &Value) -> Vec<String> {
    servers
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|server| {
            let mut url = server["url"].as_str()?.to_string();
            if let Some(vars) = server["variables"].as_object() {
                for (name, var) in vars {
                    url = url.replace(
                        &format!("{{{name}}}"),
                        var["default"].as_str().unwrap_or(""),
                    );
                }
            }
            Some(url)
        })
        .collect()
}
pub fn import_json(bytes: &[u8]) -> Result<ImportDraft> {
    if bytes.len() > 20 * MIB as usize {
        bail!("OpenAPI document exceeds 20 MiB");
    }
    let root: Value = serde_json::from_slice(bytes).context("OpenAPI source must be valid JSON")?;
    let version = root["openapi"]
        .as_str()
        .context("Missing OpenAPI version (Swagger 2 is unsupported)")?
        .to_string();
    if !["3.0.", "3.1.", "3.2."]
        .iter()
        .any(|p| version.starts_with(p))
    {
        bail!("Unsupported OpenAPI version {version}; supported versions are 3.0, 3.1 and 3.2");
    }
    let mut servers = server_urls(&root["servers"]);
    if servers.is_empty() {
        servers.push(String::new());
    }
    let mut draft = ImportDraft {
        name: root["info"]["title"]
            .as_str()
            .unwrap_or("Imported API")
            .into(),
        version,
        servers,
        operations: vec![],
        diagnostics: vec![],
    };
    let paths = root["paths"]
        .as_object()
        .context("OpenAPI paths must be an object")?;
    for (path, item) in paths {
        let item = match deref(&root, item) {
            Ok(v) => v,
            Err(e) => {
                draft.diagnostics.push(format!("{path}: {e}"));
                continue;
            }
        };
        for method in [
            "get", "post", "put", "patch", "delete", "head", "options", "trace", "query",
        ] {
            let Some(operation) = item.get(method) else {
                continue;
            };
            let mut request = RequestDefinition {
                name: operation["summary"]
                    .as_str()
                    .or(operation["operationId"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{} {path}", method.to_uppercase())),
                method: method.to_uppercase(),
                folder: operation["tags"][0].as_str().unwrap_or("Requests").into(),
                ..Default::default()
            };
            let mut diagnostics = vec![];
            let operation_servers = server_urls(&operation["servers"]);
            let path_servers = server_urls(&item["servers"]);
            let server = operation_servers.first().or(path_servers.first());
            request.url = format!(
                "{}{}",
                server
                    .map(|s| s.trim_end_matches('/'))
                    .unwrap_or("{{env.baseUrl}}"),
                path
            );
            request.extra.insert("x-openapi".into(),json!({"version":draft.version,"path":path,"method":method,"operationId":operation["operationId"]}));
            let mut parameters = std::collections::BTreeMap::new();
            for parameter in item["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(operation["parameters"].as_array().into_iter().flatten())
            {
                match deref(&root, parameter) {
                    Ok(p) => {
                        parameters.insert(
                            (
                                p["in"].as_str().unwrap_or(""),
                                p["name"].as_str().unwrap_or(""),
                            ),
                            p,
                        );
                    }
                    Err(e) => request.blockers.push(e.to_string()),
                }
            }
            for ((location, name), parameter) in parameters {
                let schema = match deref(&root, &parameter["schema"]) {
                    Ok(s) => s,
                    Err(e) => {
                        request.blockers.push(e.to_string());
                        continue;
                    }
                };
                let example = parameter
                    .get("example")
                    .cloned()
                    .or_else(|| {
                        parameter["examples"]
                            .as_object()
                            .and_then(|v| v.values().next())
                            .and_then(|v| v.get("value"))
                            .cloned()
                    })
                    .or_else(|| schema.get("example").cloned())
                    .or_else(|| schema.get("default").cloned())
                    .or_else(|| schema["enum"].as_array().and_then(|a| a.first()).cloned());
                let style = parameter["style"]
                    .as_str()
                    .unwrap_or(if location == "query" {
                        "form"
                    } else {
                        "simple"
                    });
                // `form` is the only style whose default is to explode.
                let explode = parameter["explode"].as_bool().unwrap_or(style == "form");
                let array = schema["type"] == "array";
                // A referenced item schema still has to be inspected: an array of objects has no
                // delimited form, so it must be blocked rather than emitted as a blank row.
                let items = match deref(&root, &schema["items"]) {
                    Ok(items) => items,
                    Err(e) => {
                        request.blockers.push(e.to_string());
                        continue;
                    }
                };
                let nested = array
                    && (matches!(items["type"].as_str(), Some("object" | "array"))
                        || items.get("properties").is_some());
                // Only combinations this build can reproduce byte for byte are accepted; the rest
                // stay blocked, because a silently wrong query is worse than a manual correction.
                let supported = !nested
                    && schema["type"] != "object"
                    && parameter.get("content").is_none()
                    && parameter["allowReserved"] != true
                    && match (location, style) {
                        ("query", "form") => true,
                        ("query", "spaceDelimited" | "pipeDelimited") => array && !explode,
                        ("path" | "header", "simple") => true,
                        _ => false,
                    };
                if !supported {
                    request.blockers.push(format!("{name}: unsupported structured parameter serialization; replace it manually"));
                    continue;
                }
                let delimiter = match style {
                    "spaceDelimited" => " ",
                    "pipeDelimited" => "|",
                    _ => ",",
                };
                // An exploded form array becomes one row per item; every other supported array
                // style joins into a single delimited value.
                let values = if array {
                    match example.as_ref().and_then(list) {
                        Some(items) if location == "query" && style == "form" && explode => items,
                        Some(items) => vec![items.join(delimiter)],
                        None => vec![String::new()],
                    }
                } else {
                    vec![example.as_ref().and_then(scalar).unwrap_or_default()]
                };
                match location {
                    "path" => {
                        let value = values.join(delimiter);
                        request.url = request
                            .url
                            .replace(&format!("{{{name}}}"), &format!("{{{{request.{name}}}}}"));
                        request.variables.insert(name.into(), value.clone());
                        if value.is_empty() {
                            diagnostics.push(format!("Needs input: path variable {name}"));
                        }
                    }
                    "query" | "header" => {
                        for value in values {
                            let mut row = Row::new(name, &value);
                            row.enabled = parameter["required"] == true || example.is_some();
                            if value.is_empty() && parameter["required"] == true {
                                request.variables.insert(name.into(), String::new());
                                row.value = format!("{{{{request.{name}}}}}");
                                diagnostics.push(format!("Needs input: {name}"));
                            }
                            row.extra.insert(
                                "x-openapi".into(),
                                json!({"style":style,"explode":explode}),
                            );
                            if location == "query" {
                                request.query.push(row);
                            } else {
                                request.headers.push(row);
                            }
                        }
                    }
                    _ => request
                        .blockers
                        .push(format!("{name}: unsupported parameter location {location}")),
                }
            }
            let mut auth_options = vec![];
            if let Some(security) = operation
                .get("security")
                .or(root.get("security"))
                .and_then(Value::as_array)
            {
                for requirement in security {
                    let mut auth = Auth::default();
                    let mut labels = vec![];
                    let mut blockers = vec![];
                    if let Some(requirement) = requirement.as_object() {
                        for key in requirement.keys() {
                            let scheme = &root["components"]["securitySchemes"][key];
                            match (scheme["type"].as_str(), scheme["scheme"].as_str()) {
                                (Some("http"), Some(s)) if s.eq_ignore_ascii_case("bearer") => {
                                    if auth.bearer.is_some() {
                                        blockers.push("Multiple bearer requirements need manual configuration".into());
                                    }
                                    auth.bearer = Some(SecretBinding {
                                        secret: key.clone(),
                                    });
                                    labels.push(format!("Bearer: {key}"));
                                }
                                (Some("oauth2" | "openIdConnect"), _) => {
                                    auth.bearer = Some(SecretBinding {
                                        secret: key.clone(),
                                    });
                                    labels.push(format!("Manual bearer: {key}"));
                                    diagnostics.push("Supply an externally acquired token; token acquisition is not provided".into());
                                }
                                (Some("apiKey"), _) if scheme["in"] == "header" => {
                                    if auth.api_key.is_some() {
                                        blockers
                                            .push("Multiple API keys need manual headers".into());
                                    }
                                    auth.api_key = Some(ApiKey {
                                        header: scheme["name"].as_str().unwrap_or("").into(),
                                        secret: key.clone(),
                                    });
                                    labels.push(format!("API key: {key}"));
                                }
                                _ => blockers
                                    .push(format!("Unsupported authentication scheme: {key}")),
                            }
                        }
                    }
                    auth_options.push((
                        if labels.is_empty() {
                            "No authentication".into()
                        } else {
                            labels.join(" + ")
                        },
                        auth,
                        blockers,
                    ));
                }
            }
            if auth_options.is_empty() {
                auth_options.push(("No authentication".into(), Auth::default(), vec![]));
            }
            request.auth = auth_options[0].1.clone();
            let mut bodies = vec![];
            if let Some(body) = operation.get("requestBody") {
                match deref(&root, body) {
                    Err(e) => request.blockers.push(e.to_string()),
                    Ok(body) => {
                        if let Some(content) = body["content"].as_object() {
                            for (media, definition) in content {
                                let mut examples = vec![];
                                if let Some(named) = definition["examples"].as_object() {
                                    for (name, example) in named {
                                        match deref(&root, example) {
                                            Ok(example) => {
                                                if let Some(v) = example.get("value") {
                                                    examples.push((
                                                        format!("{media} / {name} · From example"),
                                                        v.clone(),
                                                    ));
                                                } else {
                                                    diagnostics.push(format!(
                                                        "External example {name} is not fetched"
                                                    ));
                                                }
                                            }
                                            Err(e) => diagnostics.push(e.to_string()),
                                        }
                                    }
                                }
                                if let Some(example) = definition.get("example") {
                                    examples
                                        .push((format!("{media} · From example"), example.clone()));
                                }
                                if examples.is_empty() {
                                    match variants(&root, &definition["schema"]) {
                                        Ok(list) => {
                                            examples.extend(list.into_iter().map(|(label, v)| {
                                                let label = if label.is_empty() {
                                                    "Generated placeholder".to_string()
                                                } else {
                                                    format!("Generated from {label}")
                                                };
                                                (format!("{media} · {label}"), v)
                                            }))
                                        }
                                        Err(e) => {
                                            diagnostics.push(e.to_string());
                                            examples.push((
                                                format!("{media} · Supply body manually"),
                                                Value::Null,
                                            ));
                                            request.blockers.push(format!(
                                                "{media}: body example needs manual input"
                                            ));
                                        }
                                    }
                                }
                                for (label, value) in examples {
                                    let body = if media.contains("json") {
                                        Body::Json {
                                            text: serde_json::to_string_pretty(&value)?,
                                        }
                                    } else if media == "application/x-www-form-urlencoded" {
                                        Body::Form {
                                            rows: value
                                                .as_object()
                                                .into_iter()
                                                .flatten()
                                                .map(|(n, v)| {
                                                    Row::new(
                                                        n,
                                                        scalar(v).unwrap_or_else(|| v.to_string()),
                                                    )
                                                })
                                                .collect(),
                                        }
                                    } else if media.starts_with("text/") {
                                        Body::Text {
                                            text: scalar(&value)
                                                .unwrap_or_else(|| value.to_string()),
                                        }
                                    } else {
                                        request.blockers.push(format!(
                                            "Body media type {media} requires manual configuration"
                                        ));
                                        Body::None
                                    };
                                    bodies.push((label, body));
                                }
                            }
                        }
                    }
                }
            }
            if let Some((_, body)) = bodies.first() {
                request.body = body.clone();
            }
            if operation.get("callbacks").is_some() {
                diagnostics.push("Callbacks are not imported".into());
            }
            draft.operations.push(Operation {
                selected: true,
                request,
                diagnostics,
                bodies,
                body_index: 0,
                auth_options,
                auth_index: 0,
            });
        }
    }
    if root.get("webhooks").is_some() {
        draft.diagnostics.push("Webhooks are not imported".into());
    }
    if draft.operations.is_empty() {
        bail!(
            "No supported operations were found. {}",
            draft.diagnostics.join("; ")
        );
    }
    Ok(draft)
}
impl Operation {
    pub fn finish(&self) -> RequestDefinition {
        let mut request = self.request.clone();
        if let Some((_, body)) = self.bodies.get(self.body_index) {
            request.body = body.clone();
        }
        if let Some((_, auth, blockers)) = self.auth_options.get(self.auth_index) {
            request.auth = auth.clone();
            request.blockers.extend(blockers.clone());
        }
        request
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_versions_import_with_missing_path_values_and_no_tests() {
        for version in ["3.0.4", "3.1.2", "3.2.0"] {
            let root = json!({"openapi":version,"info":{"title":"API"},"servers":[{"url":"https://example.test"}],"paths":{"/users/{id}":{"parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}}],"get":{"operationId":"getUser"}}}});
            let draft = import_json(&serde_json::to_vec(&root).unwrap()).unwrap();
            assert_eq!(draft.operations.len(), 1);
            let r = draft.operations[0].finish();
            assert!(r.url.contains("{{request.id}}"));
            assert!(r.tests.file.is_empty());
            assert_eq!(draft.operations[0].diagnostics.len(), 1);
        }
    }
    #[test]
    fn security_alternatives_are_not_flattened() {
        let root = json!({"openapi":"3.1.0","paths":{"/":{"get":{"security":[{"bearer":[],"key":[]},{}]}}},"components":{"securitySchemes":{"bearer":{"type":"http","scheme":"bearer"},"key":{"type":"apiKey","in":"header","name":"X-Key"}}}});
        let draft = import_json(&serde_json::to_vec(&root).unwrap()).unwrap();
        let op = &draft.operations[0];
        assert_eq!(op.auth_options.len(), 2);
        assert!(op.finish().auth.api_key.is_some());
        assert!(op.finish().auth.bearer.is_some());
    }
    #[test]
    fn references_examples_and_readonly_fields() {
        let root = json!({"openapi":"3.2.0","paths":{"/":{"post":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Item"}}}}}}},"components":{"schemas":{"Item":{"type":"object","properties":{"id":{"type":"integer","readOnly":true},"name":{"type":"string","default":"Duck"}}}}}});
        let draft = import_json(&serde_json::to_vec(&root).unwrap()).unwrap();
        if let Body::Json { text } = draft.operations[0].finish().body {
            assert!(!text.contains("id"));
            assert!(text.contains("Duck"));
        } else {
            panic!("Expected JSON");
        }
    }
    fn query_parameter(extra: Value) -> RequestDefinition {
        let mut parameter = json!({"name":"tags","in":"query","example":["a","b"],"schema":{"type":"array","items":{"type":"string"}}});
        for (key, value) in extra.as_object().unwrap() {
            parameter[key] = value.clone();
        }
        let root = json!({"openapi":"3.1.0","paths":{"/":{"get":{"parameters":[parameter]}}}});
        import_json(&serde_json::to_vec(&root).unwrap())
            .unwrap()
            .operations[0]
            .finish()
    }
    #[test]
    fn exploded_form_arrays_become_one_row_per_item() {
        let request = query_parameter(json!({}));
        assert!(request.blockers.is_empty(), "{:?}", request.blockers);
        let rows: Vec<_> = request
            .query
            .iter()
            .map(|r| (r.name.as_str(), r.value.as_str(), r.enabled))
            .collect();
        assert_eq!(rows, vec![("tags", "a", true), ("tags", "b", true)]);
    }
    #[test]
    fn unexploded_arrays_join_with_their_style_delimiter() {
        for (style, joined) in [
            (json!({"explode":false}), "a,b"),
            (json!({"style":"pipeDelimited","explode":false}), "a|b"),
            (json!({"style":"spaceDelimited","explode":false}), "a b"),
        ] {
            let request = query_parameter(style.clone());
            assert!(
                request.blockers.is_empty(),
                "{style} -> {:?}",
                request.blockers
            );
            assert_eq!(request.query.len(), 1, "{style}");
            assert_eq!(request.query[0].value, joined, "{style}");
        }
    }
    #[test]
    fn delimited_styles_and_nested_items_stay_blocked() {
        // spaceDelimited and pipeDelimited have no defined exploded form.
        for extra in [
            json!({"style":"spaceDelimited","explode":true}),
            json!({"style":"pipeDelimited","explode":true}),
            json!({"style":"matrix","explode":false}),
            json!({"style":"deepObject","explode":true}),
        ] {
            assert!(
                !query_parameter(extra.clone()).blockers.is_empty(),
                "{extra} should be blocked"
            );
        }
        // Inline object items and referenced ones must both be rejected.
        for items in [
            json!({"type":"object"}),
            json!({"$ref":"#/components/schemas/Tag"}),
        ] {
            let root = json!({"openapi":"3.1.0","paths":{"/":{"get":{"parameters":[{"name":"tags","in":"query","schema":{"type":"array","items":items}}]}}},"components":{"schemas":{"Tag":{"properties":{"id":{"type":"string"}}}}}});
            let draft = import_json(&serde_json::to_vec(&root).unwrap()).unwrap();
            assert!(
                !draft.operations[0].finish().blockers.is_empty(),
                "{items} items should be blocked"
            );
        }
    }
    #[test]
    fn simple_path_arrays_bind_one_comma_joined_variable() {
        let root = json!({"openapi":"3.1.0","paths":{"/tags/{tags}":{"get":{"parameters":[{"name":"tags","in":"path","required":true,"example":["a","b"],"schema":{"type":"array","items":{"type":"string"}}}]}}}});
        let request = import_json(&serde_json::to_vec(&root).unwrap())
            .unwrap()
            .operations[0]
            .finish();
        assert!(request.blockers.is_empty(), "{:?}", request.blockers);
        assert!(
            request.url.ends_with("/tags/{{request.tags}}"),
            "{}",
            request.url
        );
        assert_eq!(
            request.variables.get("tags").map(String::as_str),
            Some("a,b")
        );
    }
    fn body_of(schema: Value) -> (Vec<String>, Vec<String>) {
        body_of_with(
            schema,
            json!({"schemas":{"Named":{"type":"object","properties":{"extra":{"type":"string"}}}}}),
        )
    }
    fn body_of_with(schema: Value, components: Value) -> (Vec<String>, Vec<String>) {
        let root = json!({"openapi":"3.1.0","components":components,"paths":{"/":{"post":{"requestBody":{"content":{"application/json":{"schema":schema}}}}}}});
        let draft = import_json(&serde_json::to_vec(&root).unwrap()).unwrap();
        let op = &draft.operations[0];
        let texts = op
            .bodies
            .iter()
            .map(|(_, body)| match body {
                Body::Json { text } => text.clone(),
                _ => String::new(),
            })
            .collect();
        (
            op.bodies.iter().map(|(label, _)| label.clone()).collect(),
            texts,
        )
    }
    #[test]
    fn all_of_merges_every_branch_and_the_schema_s_own_properties() {
        let (_, bodies) = body_of(json!({
            "allOf": [
                {"type":"object","properties":{"id":{"type":"integer"},"secret":{"type":"string","readOnly":true}}},
                {"$ref":"#/components/schemas/Named"}
            ],
            "properties": {"own": {"type":"string","default":"mine"}}
        }));
        assert_eq!(bodies.len(), 1);
        let text = &bodies[0];
        for expected in ["\"id\"", "extra", "mine"] {
            assert!(text.contains(expected), "{expected} missing from {text}");
        }
        // readOnly properties must never reach a request body, composed or not.
        assert!(!text.contains("secret"), "{text}");
    }
    #[test]
    fn a_branch_that_cannot_be_resolved_blocks_rather_than_merging_what_is_left() {
        let root = json!({"openapi":"3.1.0","paths":{"/":{"post":{"requestBody":{"content":{"application/json":{"schema":{"allOf":[{"type":"object","properties":{"id":{"type":"integer"}}},{"$ref":"#/components/schemas/Missing"}]}}}}}}}});
        let draft = import_json(&serde_json::to_vec(&root).unwrap()).unwrap();
        let request = draft.operations[0].finish();
        assert!(
            !request.blockers.is_empty(),
            "an unresolvable branch leaves the body shape unknown"
        );
    }
    #[test]
    fn one_of_becomes_selectable_bodies_rather_than_a_blocker() {
        let (labels, bodies) = body_of(json!({
            "oneOf": [
                {"title":"Card","type":"object","properties":{"pan":{"type":"string"}}},
                {"type":"object","properties":{"iban":{"type":"string"}}}
            ]
        }));
        assert_eq!(bodies.len(), 2, "one candidate per branch");
        assert!(labels[0].contains("Card"), "{labels:?}");
        assert!(labels[1].contains("oneOf option 2"), "{labels:?}");
        assert!(bodies[0].contains("pan"));
        assert!(bodies[1].contains("iban"));
        // A composed body is no longer a reason to block Send.
        let root = json!({"openapi":"3.1.0","paths":{"/":{"post":{"requestBody":{"content":{"application/json":{"schema":{"anyOf":[{"type":"object"}]}}}}}}}});
        let draft = import_json(&serde_json::to_vec(&root).unwrap()).unwrap();
        assert!(draft.operations[0].finish().blockers.is_empty());
    }
    #[test]
    fn unsupported_serialization_blocks_send() {
        let root = json!({"openapi":"3.1.0","paths":{"/":{"get":{"parameters":[{"name":"filter","in":"query","style":"deepObject","schema":{"type":"object"}}]}}}});
        let draft = import_json(&serde_json::to_vec(&root).unwrap()).unwrap();
        assert!(!draft.operations[0].finish().blockers.is_empty());
    }
}
