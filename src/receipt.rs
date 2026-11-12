//! The receipt of an icon import: what Steam took of each icon, kept beside the files.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::achievements::{Drift, icon_word};
use crate::editor::Icon;
use crate::webapi::Achievement;

/// The file in the folder of an import that records, once Steam has taken it, each icon it took.
/// The editor imports nothing of this name, so `--write` leaves it alone.
pub const RECEIPT: &str = "steam-icons.json";

/// The image Steam made of an icon, by the name it serves it under at the end of its `address`.
/// Steam serves one image from more than one host, so the name alone tells one from another.
#[must_use]
pub fn steam_image(address: &str) -> &str {
    address.rsplit_once('/').map_or(address, |(_, name)| name)
}

/// One icon as an import left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The SHA-256 of the file imported, in hexadecimal.
    sha256: String,
    /// The image Steam made of it, as [`steam_image`] names it.
    steam: String,
}

impl Imported {
    /// Whether the file of `icon` is another than the one imported.
    #[must_use]
    pub fn file_changed(&self, icon: &Icon) -> bool {
        self.sha256 != icon.sha256()
    }

    /// Whether the image Steam serves at `address` is still the one this import made.
    #[must_use]
    pub fn still_on_steam(&self, address: &str) -> bool {
        self.steam == steam_image(address)
    }
}

/// What an import left on Steam of each icon, by the name of its file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Receipt {
    pub app: u32,
    pub icons: BTreeMap<String, Imported>,
}

impl Receipt {
    /// The `icons` as Steam took them for `app`, with the address each has in `held`.
    #[must_use]
    pub fn of(app: u32, icons: &[Icon], held: &[Achievement]) -> Self {
        Self {
            app,
            icons: icons
                .iter()
                .map(|icon| {
                    (
                        icon.name.clone(),
                        Imported {
                            sha256: icon.sha256(),
                            steam: steam_image(icon.on_steam(held)).to_owned(),
                        },
                    )
                })
                .collect(),
        }
    }

    /// The receipt as it is kept: JSON, a key to a line in a set order, so that a change to one
    /// icon changes its lines alone.
    #[must_use]
    pub fn text(&self) -> String {
        let icons: Map<String, Value> = self
            .icons
            .iter()
            .map(|(name, imported)| {
                (
                    name.clone(),
                    json!({"sha256": imported.sha256, "steam": imported.steam}),
                )
            })
            .collect();
        let text = serde_json::to_string_pretty(&json!({"app": self.app, "icons": icons}))
            .unwrap_or_default();
        format!("{text}\n")
    }

    /// The receipt kept as `text`.
    ///
    /// # Errors
    ///
    /// When it is not one.
    pub fn read(text: &str) -> Result<Self, String> {
        let kept: Value =
            serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
        let app = kept
            .get("app")
            .and_then(Value::as_u64)
            .and_then(|app| u32::try_from(app).ok())
            .ok_or("\"app\" is not an app ID")?;
        let icons = kept
            .get("icons")
            .and_then(Value::as_object)
            .ok_or("there is no \"icons\" object")?
            .iter()
            .map(|(name, imported)| {
                let field = |key: &str| {
                    imported
                        .get(key)
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or_else(|| format!("{name} has no \"{key}\" text"))
                };
                Ok((
                    name.clone(),
                    Imported {
                        sha256: field("sha256")?,
                        steam: field("steam")?,
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self { app, icons })
    }

    /// How each of the `icons` differs from what the receipt says Steam took of it, as Steam's
    /// achievements, `held`, now give it. An icon Steam has none of is left to the check that
    /// names it so.
    #[must_use]
    pub fn drift(&self, icons: &[Icon], held: &[Achievement]) -> Vec<Drift> {
        icons
            .iter()
            .filter_map(|icon| {
                let address = icon.on_steam(held);
                if address.is_empty() {
                    return None;
                }
                let which = icon_word(icon.locked);
                let how = match self.icons.get(&icon.name) {
                    None => format!(
                        "the {which} has no import in {RECEIPT}. Import it, publish, then run \
                         --imported"
                    ),
                    Some(imported) if imported.file_changed(icon) => format!(
                        "the {which} changed in the files since the import {RECEIPT} records. \
                         Import it, publish, then run --imported"
                    ),
                    Some(imported) if !imported.still_on_steam(address) => format!(
                        "the {which} on Steam changed since the import {RECEIPT} records, and \
                         not from the files. Import it again, publish, then run --imported"
                    ),
                    Some(_) => return None,
                };
                Some(Drift::Worded {
                    api_name: icon.api_name.clone(),
                    how,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::icon_name;

    /// An achievement on Steam whose icons are at `icon` and `icon_locked`.
    fn held(api_name: &str, icon: &str, icon_locked: &str) -> Achievement {
        Achievement {
            api_name: api_name.to_owned(),
            name: String::new(),
            description: String::new(),
            hidden: false,
            icon: icon.to_owned(),
            icon_locked: icon_locked.to_owned(),
        }
    }

    fn icon(api_name: &str, locked: bool, bytes: &[u8]) -> Icon {
        Icon {
            api_name: api_name.to_owned(),
            locked,
            name: icon_name(api_name, locked),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn a_receipt_records_each_icons_file_and_steams_image_and_reads_back_the_same() {
        let icons = [icon("A", false, b"a"), icon("A", true, b"b")];
        let steam = [held("A", "https://cdn/1/aa.jpg", "https://cdn/1/bb.jpg")];
        let receipt = Receipt::of(1_000_050, &icons, &steam);
        let text = receipt.text();
        assert_eq!(
            text,
            "{\n  \"app\": 1000050,\n  \"icons\": {\n    \"A_ACHIEVED.png\": {\n      \
             \"sha256\": \"ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb\",\n      \
             \"steam\": \"aa.jpg\"\n    },\n    \"A_UNACHIEVED.png\": {\n      \
             \"sha256\": \"3e23e8160039594a33894f6564e1b1348bbd7a0088d42c4acb73eeaed59c009d\",\n      \
             \"steam\": \"bb.jpg\"\n    }\n  }\n}\n"
        );
        assert_eq!(Receipt::read(&text), Ok(receipt));
    }

    #[test]
    fn a_receipt_that_is_not_one_is_refused_saying_why() {
        for (text, why) in [
            ("[", "not JSON: EOF while parsing a list at line 1 column 1"),
            ("{\"icons\": {}}", "\"app\" is not an app ID"),
            (
                "{\"app\": 4294967296, \"icons\": {}}",
                "\"app\" is not an app ID",
            ),
            ("{\"app\": 1}", "there is no \"icons\" object"),
            (
                "{\"app\": 1, \"icons\": {\"A_ACHIEVED.png\": {\"sha256\": \"00\"}}}",
                "A_ACHIEVED.png has no \"steam\" text",
            ),
            (
                "{\"app\": 1, \"icons\": {\"A_ACHIEVED.png\": {\"steam\": \"a.jpg\"}}}",
                "A_ACHIEVED.png has no \"sha256\" text",
            ),
        ] {
            assert_eq!(Receipt::read(text), Err(why.to_owned()), "{text}");
        }
    }

    #[test]
    fn an_icon_moved_from_the_recorded_import_is_named_and_one_steam_lacks_is_left_alone() {
        let imported = [
            icon("SAME", false, b"same"),
            icon("FILE", false, b"old"),
            icon("STEAM", true, b"steam"),
            icon("BARE", false, b"bare"),
        ];
        let before = [
            held("SAME", "https://cdn/s.jpg", ""),
            held("FILE", "https://cdn/f.jpg", ""),
            held("STEAM", "", "https://cdn/t.jpg"),
            held("BARE", "https://cdn/b.jpg", ""),
        ];
        let receipt = Receipt::of(1, &imported, &before);
        let now = [
            icon("SAME", false, b"same"),
            icon("FILE", false, b"new"),
            icon("STEAM", true, b"steam"),
            icon("BARE", false, b"bare"),
            icon("NEW", false, b"new"),
            icon("UNIMPORTED", false, b"unimported"),
        ];
        let after = [
            held("SAME", "https://other-host/s.jpg", ""),
            held("FILE", "https://cdn/f.jpg", ""),
            held("STEAM", "", "https://cdn/u.jpg"),
            held("BARE", "", ""),
            held("NEW", "https://cdn/n.jpg", ""),
        ];
        let named: Vec<String> = receipt
            .drift(&now, &after)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            named,
            [
                "FILE: the icon changed in the files since the import steam-icons.json records. \
                 Import it, publish, then run --imported",
                "STEAM: the locked icon on Steam changed since the import steam-icons.json \
                 records, and not from the files. Import it again, publish, then run --imported",
                "NEW: the icon has no import in steam-icons.json. Import it, publish, then run \
                 --imported",
            ]
        );
    }

    #[test]
    fn steams_image_is_named_by_the_end_of_its_address() {
        assert_eq!(steam_image("https://cdn/apps/1/ab.jpg"), "ab.jpg");
        assert_eq!(steam_image("ab.jpg"), "ab.jpg");
    }

    #[test]
    fn an_icon_entry_that_is_not_an_object_is_refused() {
        let refused = Receipt::read(r#"{"app":1,"icons":{"a":7}}"#);
        assert_eq!(refused, Err("a has no \"sha256\" text".to_owned()));
    }

    #[test]
    fn an_icon_name_holding_a_double_quote_is_kept_and_read_back() {
        let receipt =
            Receipt::read(r#"{"app":1,"icons":{"a\"b":{"sha256":"","steam":""}}}"#).unwrap();
        assert!(receipt.icons.contains_key("a\"b"));
        assert_eq!(Receipt::read(&receipt.text()), Ok(receipt));
    }
}
