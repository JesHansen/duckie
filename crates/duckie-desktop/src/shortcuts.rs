use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use std::sync::OnceLock;

pub struct Binding {
    pub id: &'static str,
    pub label: &'static str,
    pub action: &'static str,
    pub context: &'static str,
    pub chord: KeyboardShortcut,
}
pub fn bindings() -> &'static [Binding] {
    static TABLE: OnceLock<Vec<Binding>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("shortcuts.tsv")
            .lines()
            .skip(1)
            .map(|line| {
                let fields: Vec<_> = line.split('\t').collect();
                assert_eq!(fields.len(), 4, "Invalid shortcut row");
                let mut parts = fields[1].split('+').collect::<Vec<_>>();
                let key = Key::from_name(parts.pop().unwrap()).expect("Invalid shortcut key");
                let mut modifiers = Modifiers::NONE;
                for part in parts {
                    modifiers |= match part {
                        "Ctrl" => Modifiers::CTRL,
                        "Shift" => Modifiers::SHIFT,
                        "Alt" => Modifiers::ALT,
                        _ => panic!("Invalid shortcut modifier"),
                    };
                }
                Binding {
                    id: fields[0],
                    label: fields[1],
                    action: fields[2],
                    context: fields[3],
                    chord: KeyboardShortcut::new(modifiers, key),
                }
            })
            .collect()
    })
}
pub fn binding(id: &str) -> &'static Binding {
    bindings()
        .iter()
        .find(|b| b.id == id)
        .expect("Unknown shortcut ID")
}
pub fn pressed(ctx: &egui::Context, id: &str) -> bool {
    ctx.input_mut(|input| input.consume_shortcut(&binding(id).chord))
}
pub fn consume(input: &mut egui::InputState, id: &str) -> bool {
    let chord = &binding(id).chord;
    input.consume_key(chord.modifiers, chord.logical_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_is_unique_and_readme_matches_the_binding_table() {
        let mut ids = std::collections::BTreeSet::new();
        let mut table = String::from("| Shortcut | Action | Context |\n| --- | --- | --- |\n");
        for b in bindings() {
            assert!(ids.insert(b.id));
            table.push_str(&format!(
                "| **{}** | {} | {} |\n",
                b.label, b.action, b.context
            ));
        }
        let readme = include_str!("../../../README.md").replace("\r\n", "\n");
        let generated = readme
            .split("<!-- shortcuts:start -->\n")
            .nth(1)
            .unwrap()
            .split("<!-- shortcuts:end -->")
            .next()
            .unwrap();
        assert_eq!(generated, table);
    }
}
