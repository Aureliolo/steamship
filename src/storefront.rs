//! What Steam's public store page shows of an app in each language: its system requirements.
//!
//! Steam shows them in its store's app details, as HTML, once the store page is public, Coming
//! Soon included, and nowhere a program can read before. Steam rewrites the parts of a requirement
//! given as a choice or an amount, so only those typed as text are compared word for word.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::Value as Json;

use crate::text::Requirement;
use crate::webapi;

const HOST: &str = "https://store.steampowered.com";

/// Each platform's key in the app details, by the platform's name on the Basic Info form.
const PLATFORMS: [(&str, &str); 3] = [
    ("windows", "pc_requirements"),
    ("mac", "mac_requirements"),
    ("linux", "linux_requirements"),
];

/// Each tier's key in the app details, by the tier's name on the Basic Info form.
const TIERS: [(&str, &str); 2] = [("min", "minimum"), ("rec", "recommended")];

/// The fields the store page repeats as typed. It writes memory and storage with their units, the
/// DirectX version as "Version 11" and VR support as its own words, each in the page's language.
const TYPED: [&str; 5] = ["osversion", "processor", "graphics", "soundcard", "notes"];

/// The size an answer has to stay under to be read. Steam's app details run to tens of kilobytes.
const LIMIT: u64 = 8 * 1024 * 1024;

/// The lines a store page shows, by platform and tier as the Basic Info form names them.
pub type Shown = BTreeMap<(String, String), Vec<String>>;

/// Steam's public store.
#[derive(Debug)]
pub struct Storefront {
    agent: ureq::Agent,
    host: String,
}

impl Default for Storefront {
    fn default() -> Self {
        Self::new()
    }
}

impl Storefront {
    #[must_use]
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            agent,
            host: webapi::host_or_stand_in(HOST),
        }
    }

    /// The system requirements `app`'s store page shows in `language`, as Steam names it, such as
    /// `german`; `None` while Steam shows no public page for it.
    ///
    /// # Errors
    ///
    /// When the store cannot be reached or answers with anything but app details.
    pub fn requirements(&self, app: u32, language: &str) -> Result<Option<Shown>, String> {
        let started = Instant::now();
        let mut response = self
            .agent
            .get(format!("{}/api/appdetails", self.host))
            .query("appids", app.to_string())
            .query("l", language)
            .call()
            .map_err(|error| {
                let unreached = webapi::Unreached::from(&error, started.elapsed());
                format!("Steam's store could not be reached: {unreached}")
            })?;
        let status = response.status().as_u16();
        if !(200..=299).contains(&status) {
            return Err(format!("Steam's store answered HTTP {status}"));
        }
        let body = response
            .body_mut()
            .with_config()
            .limit(LIMIT)
            .read_to_string()
            .map_err(|error| format!("Steam's store's answer could not be read whole: {error}"))?;
        shown(&body, app)
    }
}

/// What the app details in `body` show of `app`'s system requirements; `None` when they say there
/// is no public page.
///
/// # Errors
///
/// When `body` is not app details for `app`.
pub fn shown(body: &str, app: u32) -> Result<Option<Shown>, String> {
    let unread = || format!("Steam's store answered with no details of app {app}");
    let answer = serde_json::from_str::<Json>(body).ok().ok_or_else(unread)?;
    let details = answer.get(app.to_string()).ok_or_else(unread)?;
    match details.get("success").and_then(Json::as_bool) {
        Some(true) => {}
        Some(false) => return Ok(None),
        None => return Err(unread()),
    }
    let data = details.get("data").ok_or_else(unread)?;
    let mut found = Shown::new();
    for (platform, key) in PLATFORMS {
        // A platform with no requirements comes as an empty list rather than an object.
        let Some(tiers) = data.get(key).and_then(Json::as_object) else {
            continue;
        };
        found.extend(TIERS.iter().filter_map(|(tier, named)| {
            let html = tiers.get(*named)?.as_str()?;
            Some(((platform.to_owned(), (*tier).to_owned()), lines(html)))
        }));
    }
    Ok(Some(found))
}

/// Each line of a list of requirements as the store page marks it up, without its label:
/// `<li><strong>OS:</strong> Windows 10<br></li>` is `Windows 10`.
#[must_use]
pub fn lines(html: &str) -> Vec<String> {
    html.split("<li>")
        .skip(1)
        .map(|item| {
            let item = item.split_once("</li>").map_or(item, |(line, _)| line);
            let item = item.strip_suffix("<br>").unwrap_or(item);
            let value = item
                .split_once("</strong>")
                .map_or(item, |(_, value)| value);
            unescaped(value.trim())
        })
        .collect()
}

/// `text` with the HTML entities the store page writes as the characters they stand for.
fn unescaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        let entity = from
            .get(1..)
            .and_then(|after| after.split_once(';'))
            .and_then(|(name, after)| character(name).map(|found| (found, after)));
        if let Some((found, after)) = entity {
            out.push(found);
            rest = after;
        } else {
            out.push('&');
            rest = from.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    out
}

/// The character an entity's `name`, between `&` and `;`, stands for.
fn character(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Whether the store page shows `wanted` as typed, so that it can be compared word for word.
#[must_use]
pub fn comparable(wanted: &Requirement) -> bool {
    PLATFORMS
        .iter()
        .any(|(platform, _)| *platform == wanted.platform)
        && TIERS.iter().any(|(tier, _)| *tier == wanted.tier)
        && TYPED.contains(&wanted.field.as_str())
}

/// Whether `shown` holds `wanted`'s text as one of its platform's and tier's lines.
#[must_use]
pub fn holds(shown: &Shown, wanted: &Requirement) -> bool {
    shown
        .get(&(wanted.platform.clone(), wanted.tier.clone()))
        .is_some_and(|lines| lines.iter().any(|line| line == wanted.text.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steam's app details in German, cut to the requirements, each list copied from the store's
    /// answer for a public game; Linux's as a Coming Soon page with none typed shows it.
    const DETAILS: &str = r#"{"1000010":{"success":true,"data":{"type":"game","name":"Sample",
        "pc_requirements":{"minimum":"<strong>Mindestanforderungen:</strong><br><ul class=\"bb_ul\"><li>Setzt 64-Bit-Prozessor und -Betriebssystem voraus<br></li><li><strong>Betriebssystem:</strong> Windows 10<br></li><li><strong>Prozessor:</strong> 2 GHz Intel Dual Core 64-bit Processor<br></li><li><strong>Arbeitsspeicher:</strong> 4 GB RAM<br></li><li><strong>Grafik:</strong> NVIDIA &amp; AMD (1GB VRAM)<br></li><li><strong>DirectX:</strong> Version 11<br></li></ul>",
        "recommended":"<strong>Empfohlen:</strong><br><ul class=\"bb_ul\"><li>Setzt 64-Bit-Prozessor und -Betriebssystem voraus<br></li><li><strong>Betriebssystem:</strong> Windows 10 64-bit<br></li><li><strong>Prozessor:</strong> Intel i7 8700K / AMD r5 3600<br></li></ul>"},
        "mac_requirements":{"minimum":"<strong>Mindestanforderungen:</strong><br><ul class=\"bb_ul\"><li><strong>Betriebssystem:</strong> 10.13.6+<br></li><li><strong>Prozessor:</strong> Dual Core 2.4GHz<br></li></ul>"},
        "linux_requirements":{"minimum":"<strong>Mindestanforderungen:</strong><br><ul class=\"bb_ul\"></ul>","recommended":"<strong>Empfohlen:</strong><br><ul class=\"bb_ul\"></ul>"}}}}"#;

    fn wanted(platform: &str, tier: &str, field: &str, text: &str) -> Requirement {
        Requirement {
            language: "german".to_owned(),
            platform: platform.to_owned(),
            tier: tier.to_owned(),
            field: field.to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_public_page_s_requirements_are_read_by_platform_and_tier_without_their_labels() {
        let shown = shown(DETAILS, 1_000_010).unwrap().unwrap();
        let at = |platform: &str, tier: &str| {
            shown
                .get(&(platform.to_owned(), tier.to_owned()))
                .cloned()
                .ok_or("not shown")
                .unwrap()
        };
        assert_eq!(shown.len(), 5);
        assert_eq!(
            at("windows", "min"),
            [
                "Setzt 64-Bit-Prozessor und -Betriebssystem voraus",
                "Windows 10",
                "2 GHz Intel Dual Core 64-bit Processor",
                "4 GB RAM",
                "NVIDIA & AMD (1GB VRAM)",
                "Version 11"
            ]
        );
        assert_eq!(
            at("windows", "rec"),
            [
                "Setzt 64-Bit-Prozessor und -Betriebssystem voraus",
                "Windows 10 64-bit",
                "Intel i7 8700K / AMD r5 3600"
            ]
        );
        assert_eq!(at("mac", "min"), ["10.13.6+", "Dual Core 2.4GHz"]);
        assert!(at("linux", "min").is_empty() && at("linux", "rec").is_empty());
    }

    #[test]
    fn a_page_steam_does_not_show_reads_as_none_and_anything_else_is_refused() {
        assert_eq!(
            shown(r#"{"1000010":{"success":false}}"#, 1_000_010),
            Ok(None)
        );
        for body in [
            "null",
            "{}",
            r#"{"1000030":{"success":true,"data":{}}}"#,
            "<html>",
        ] {
            assert_eq!(
                shown(body, 1_000_010),
                Err("Steam's store answered with no details of app 1000010".to_owned()),
                "{body}"
            );
        }
    }

    #[test]
    fn a_requirement_is_held_only_word_for_word_on_its_own_platform_and_tier() {
        let shown = shown(DETAILS, 1_000_010).unwrap().unwrap();
        assert!(holds(
            &shown,
            &wanted("windows", "rec", "osversion", " Windows 10 64-bit")
        ));
        assert!(holds(
            &shown,
            &wanted("windows", "min", "graphics", "NVIDIA & AMD (1GB VRAM)")
        ));
        assert!(!holds(
            &shown,
            &wanted("windows", "min", "osversion", "Windows 10 64-bit")
        ));
        assert!(!holds(
            &shown,
            &wanted("linux", "rec", "osversion", "Ubuntu 22.04")
        ));
        assert!(!holds(
            &shown,
            &wanted("mac", "rec", "osversion", "10.13.6+")
        ));
        assert!(!holds(
            &shown,
            &wanted("windows", "min", "processor", "2 GHz Intel")
        ));
    }

    #[test]
    fn only_requirements_typed_as_text_on_a_platform_the_store_shows_are_comparable() {
        assert!(comparable(&wanted("mac", "rec", "soundcard", "Any")));
        assert!(comparable(&wanted("linux", "min", "notes", "None")));
        assert!(!comparable(&wanted(
            "windows",
            "min",
            "memory][amount",
            "4"
        )));
        assert!(!comparable(&wanted("windows", "rec", "directx", "11")));
        assert!(!comparable(&wanted(
            "android",
            "min",
            "osversion",
            "Android 12"
        )));
        assert!(!comparable(&wanted(
            "windows",
            "max",
            "osversion",
            "Windows 11"
        )));
    }

    #[test]
    fn entities_are_read_as_their_characters_and_a_bare_ampersand_is_kept() {
        assert_eq!(
            unescaped("&lt;a&gt; &quot;b&quot; &apos;c&#39; &#x41;&#66; R&D &bogus; &"),
            "<a> \"b\" 'c' AB R&D &bogus; &"
        );
    }
}
