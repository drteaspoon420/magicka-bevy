//! StringCollection/LanguageManager and the text phase of Message.Initialize.
//! Font wrapping and language switching belong to the presentation host.
use crate::{
    dialog_definition::TextDefinition,
    scene_loading::custom_hash,
    scene_script::{xml, LayoutError},
    script_attributes::ScriptReadError,
};
use std::collections::BTreeMap;

fn localization_id(value: &str) -> Result<i32, LocalizationError> {
    // Retail Spanish tables contain NBSP in identifiers. It is unchanged by
    // invariant casing; other non-ASCII casing remains explicitly unsupported.
    if value.chars().any(|c| !c.is_ascii() && c != '\u{a0}') {
        return Err(LocalizationError::Identifier(
            ScriptReadError::UnsupportedName(value.into()),
        ));
    }
    Ok(custom_hash(&value.to_ascii_lowercase()))
}

#[derive(Debug, PartialEq, Eq)]
pub enum LocalizationError {
    Xml(LayoutError),
    MissingWorkbook,
    /// Native indexes all ChildNodes; this decoder handles element-only rows.
    UnsupportedRowNodes,
    Identifier(ScriptReadError),
    NullString(i32),
    MissingArguments,
    ArgumentIndex(i32),
    /// Caller-controlled guard; the original loop has no substitution limit.
    SubstitutionLimit,
}

#[derive(Debug, Default)]
pub struct LocalizationTable {
    pub strings: BTreeMap<i32, String>,
    players: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StringLookup<'a> {
    pub found: bool,
    pub value: Option<&'a str>,
}
impl LocalizationTable {
    /// Native TryGetString returns raw table values (no backslash-n expansion),
    /// and reports #players found even when its cached value is null.
    pub fn try_get_string(&self, id: i32) -> StringLookup<'_> {
        if id == custom_hash("#players") {
            return StringLookup {
                found: true,
                value: self.players.as_deref(),
            };
        }
        let value = self.strings.get(&id).map(String::as_str);
        StringLookup {
            found: value.is_some(),
            value,
        }
    }
    /// TextBox's ParseReferences hashes every token, including numeric spellings.
    /// Unlike Message.Initialize, it never indexes a message argument array.
    pub fn parse_references(
        &self,
        text: &str,
        max_substitutions: usize,
    ) -> Result<String, LocalizationError> {
        let mut value = text.to_owned();
        let mut substitutions = 0;
        while let Some(start) = value.find('#') {
            let Some(end) = value[start + 1..].find(';').map(|i| start + 1 + i) else {
                break;
            };
            if substitutions == max_substitutions {
                return Err(LocalizationError::SubstitutionLimit);
            }
            let id = localization_id(&value[start..end])?;
            let replacement = self.get_string(id).unwrap_or_default();
            value.replace_range(start..=end, &format!("[c=1,1,1]{replacement}[/c]"));
            substitutions += 1;
        }
        Ok(value)
    }
    /// Apply a file in caller order. Later nonempty values overwrite earlier ones.
    /// Returns accepted rows, including overwrites. Filesystem ordering is not inferred.
    pub fn add_xml(&mut self, bytes: &[u8]) -> Result<usize, LocalizationError> {
        let root = xml(bytes).map_err(LocalizationError::Xml)?;
        if !root.name.eq_ignore_ascii_case("workbook") {
            return Err(LocalizationError::MissingWorkbook);
        }
        let mut accepted = 0;
        for worksheet in root
            .children
            .iter()
            .filter(|n| n.name.eq_ignore_ascii_case("worksheet"))
        {
            for table in worksheet
                .children
                .iter()
                .filter(|n| n.name.eq_ignore_ascii_case("table"))
            {
                for row in table
                    .children
                    .iter()
                    .filter(|n| n.name.eq_ignore_ascii_case("row"))
                {
                    if row.non_element_children {
                        return Err(LocalizationError::UnsupportedRowNodes);
                    }
                    if row.children.len() < 2 {
                        continue;
                    }
                    let key = &row.children[0].inner_text;
                    let value = &row.children[1].inner_text;
                    if key.starts_with('#') && !value.is_empty() {
                        let id = localization_id(key)?;
                        self.strings.insert(id, value.clone());
                        accepted += 1;
                    }
                }
            }
        }
        Ok(accepted)
    }

    /// Connected player names only, in original slot order. Empty input preserves
    /// the previous cache (which starts null).
    pub fn set_player_names(&mut self, names: &[&str]) {
        if let Some((first, rest)) = names.split_first() {
            let mut result = (*first).to_owned();
            for (i, name) in rest.iter().enumerate() {
                result.push_str(if i + 1 == rest.len() { " & " } else { ", " });
                result.push_str(name);
            }
            self.players = Some(result);
        }
    }

    /// None represents the original nullable #players cache, not a missing key.
    pub fn get_string(&self, id: i32) -> Option<String> {
        if id == custom_hash("#players") {
            return self.players.clone();
        }
        Some(match self.strings.get(&id) {
            Some(value) => value.replace("\\n", "\n"),
            None => format!("NOT FOUND: {id}"),
        })
    }

    /// Resolve the pre-font text phase. Every replacement restarts the scan over
    /// the whole string, including inserted references and their white color tags.
    /// A finite budget makes malformed cycles reviewable instead of hanging the host.
    pub fn expand_message(
        &self,
        text: &TextDefinition,
        max_substitutions: usize,
    ) -> Result<String, LocalizationError> {
        let mut value = self
            .get_string(text.localization)
            .ok_or(LocalizationError::NullString(text.localization))?;
        let mut substitutions = 0;
        while let Some(start) = value.find('#') {
            let Some(end) = value[start + 1..].find(';').map(|i| start + 1 + i) else {
                break;
            };
            if substitutions == max_substitutions {
                return Err(LocalizationError::SubstitutionLimit);
            }
            let token = &value[start..end];
            let id = if let Ok(index) = token[1..].trim().parse::<i32>() {
                let args = text
                    .arguments
                    .as_ref()
                    .ok_or(LocalizationError::MissingArguments)?;
                let offset = index.wrapping_sub(1);
                let offset =
                    usize::try_from(offset).map_err(|_| LocalizationError::ArgumentIndex(index))?;
                *args
                    .get(offset)
                    .ok_or(LocalizationError::ArgumentIndex(index))?
            } else {
                localization_id(token)?
            };
            // CLR string concatenation treats a null replacement as empty; only
            // the initial null fails when Initialize calls IndexOf on it.
            let replacement = self.get_string(id).unwrap_or_default();
            value.replace_range(start..=end, &format!("[c=1,1,1]{replacement}[/c]"));
            substitutions += 1;
        }
        Ok(value)
    }
}
