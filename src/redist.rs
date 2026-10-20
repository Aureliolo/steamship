//! What a build's Windows programs and libraries load that neither Windows nor the build itself
//! gives them, and which of Steamworks' Common Redistributables installs it.
//!
//! Steam runs a ticked redistributable's installer before the game first starts, so a DLL one of
//! them installs is there for a game that ticks it. An app shows each it ticks among its own
//! depots, as a depot of app 228980, in what `app_info_print` prints and in a settings snapshot.

use std::collections::BTreeSet;
use std::path::Path;

use crate::check::Report;
use crate::scripts::Problem;
use crate::vdf::{Block, Value};

/// The app Steamworks' Common Redistributables are depots of.
const REDISTRIBUTABLES: &str = "228980";

/// The DLLs every edition of Windows 10 and later has, so that a game needs no installer for them,
/// and `vulkan-1.dll` and `opencl.dll`, which every graphics driver installs. Media Foundation is
/// left out, as the N editions lack it.
const WINDOWS_DLLS: [&str; 87] = [
    "advapi32.dll",
    "avrt.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "cfgmgr32.dll",
    "combase.dll",
    "comctl32.dll",
    "comdlg32.dll",
    "credui.dll",
    "crypt32.dll",
    "d2d1.dll",
    "d3d9.dll",
    "d3d10.dll",
    "d3d10_1.dll",
    "d3d11.dll",
    "d3d12.dll",
    "d3dcompiler_47.dll",
    "dbghelp.dll",
    "dcomp.dll",
    "dinput8.dll",
    "dnsapi.dll",
    "dsound.dll",
    "dwmapi.dll",
    "dwrite.dll",
    "dxgi.dll",
    "dxva2.dll",
    "gdi32.dll",
    "gdiplus.dll",
    "glu32.dll",
    "hid.dll",
    "imm32.dll",
    "iphlpapi.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "ksuser.dll",
    "mpr.dll",
    "msacm32.dll",
    "mscoree.dll",
    "msctf.dll",
    "msimg32.dll",
    "msvcp_win.dll",
    "msvcrt.dll",
    "mswsock.dll",
    "ncrypt.dll",
    "netapi32.dll",
    "normaliz.dll",
    "ntdll.dll",
    "ole32.dll",
    "oleacc.dll",
    "oleaut32.dll",
    "opencl.dll",
    "opengl32.dll",
    "powrprof.dll",
    "propsys.dll",
    "psapi.dll",
    "rpcrt4.dll",
    "sechost.dll",
    "secur32.dll",
    "setupapi.dll",
    "shcore.dll",
    "shell32.dll",
    "shlwapi.dll",
    "sspicli.dll",
    "ucrtbase.dll",
    "uiautomationcore.dll",
    "urlmon.dll",
    "user32.dll",
    "userenv.dll",
    "usp10.dll",
    "uxtheme.dll",
    "version.dll",
    "vulkan-1.dll",
    "windowscodecs.dll",
    "winhttp.dll",
    "wininet.dll",
    "winmm.dll",
    "winspool.drv",
    "wintrust.dll",
    "winusb.dll",
    "wldap32.dll",
    "ws2_32.dll",
    "wsock32.dll",
    "wtsapi32.dll",
    "xaudio2_8.dll",
    "xaudio2_9.dll",
    "xinput1_4.dll",
    "xinput9_1_0.dll",
];

/// Windows' API sets, which name no file: Windows itself says which DLL answers each.
const API_SETS: [&str; 2] = ["api-ms-win-", "ext-ms-win-"];

/// Each redistributable's depot in app 228980, by the name Steamworks gives it.
const NAMES: [(u32, &str); 15] = [
    (228_981, "VC 2005 Redist"),
    (228_982, "VC 2008 Redist"),
    (228_983, "VC 2010 Redist"),
    (228_984, "VC 2012 Redist"),
    (228_985, "VC 2013 Redist"),
    (228_986, "VC 2015 Redist"),
    (228_987, "VC 2017 Redist"),
    (228_988, "VC 2019 Redist"),
    (228_989, "VC 2022 Redist"),
    (228_990, "DirectX Jun 2010 Redist"),
    (229_020, "OpenAL 2.0.7.0 Redist"),
    (229_030, "PhysX System Software 8.09.04"),
    (229_031, "PhysX System Software 9.12.1031"),
    (229_032, "PhysX System Software 9.13.1220"),
    (229_033, "PhysX System Software 9.14.0702"),
];

/// The DirectX DLLs of the June 2010 redistributable, which Windows never had, as a name and the
/// numbers it ends in: D3DX, the shader compilers before 47, `XInput` before 1.4, and the audio
/// libraries of the SDK.
const DIRECTX: [(&str, u8, u8); 11] = [
    ("d3dx9_", 24, 43),
    ("d3dx10_", 33, 43),
    ("d3dx11_", 42, 43),
    ("d3dcompiler_", 33, 43),
    ("d3dcsx_", 42, 43),
    ("xinput1_", 1, 3),
    ("x3daudio1_", 0, 7),
    ("xapofx1_", 0, 5),
    ("xaudio2_", 0, 7),
    ("xactengine2_", 0, 10),
    ("xactengine3_", 0, 7),
];

/// The depots of the redistributables that install `dll`, a lower-case file name, if any does.
///
/// Visual C++ 2015 to 2022 share one runtime, each release adding to it, so a DLL a later one
/// added is installed only by it and those after it.
#[must_use]
pub fn installed_by(dll: &str) -> Option<&'static [u32]> {
    let stem = dll.strip_suffix(".dll")?;
    Some(match stem {
        "msvcr80" | "msvcp80" | "msvcm80" | "mfc80" | "mfc80u" | "vcomp" => &[228_981],
        "msvcr90" | "msvcp90" | "msvcm90" | "mfc90" | "mfc90u" | "vcomp90" => &[228_982],
        "msvcr100" | "msvcp100" | "vcomp100" | "mfc100" | "mfc100u" => &[228_983],
        "msvcr110" | "msvcp110" | "vcomp110" | "vccorlib110" | "mfc110" | "mfc110u" => &[228_984],
        "msvcr120" | "msvcp120" | "vcomp120" | "vccorlib120" | "mfc120" | "mfc120u" => &[228_985],
        "vcruntime140" | "msvcp140" | "concrt140" | "vccorlib140" | "vcomp140" | "vcamp140"
        | "mfc140" | "mfc140u" => &[228_986, 228_987, 228_988, 228_989],
        "msvcp140_1" | "msvcp140_2" => &[228_987, 228_988, 228_989],
        "vcruntime140_1" => &[228_988, 228_989],
        "msvcp140_atomic_wait" | "msvcp140_codecvt_ids" => &[228_989],
        "openal32" | "wrap_oal" => &[229_020],
        "physxloader" | "physxcore" | "physxcooking" | "physxdevice" | "physxupdateloader" => {
            &[229_030, 229_031, 229_032, 229_033]
        }
        "d3dx10" => &[228_990],
        _ if DIRECTX.iter().any(|&(name, from, to)| {
            stem.strip_prefix(name)
                .and_then(|number| number.parse::<u8>().ok())
                .is_some_and(|number| (from..=to).contains(&number))
        }) =>
        {
            &[228_990]
        }
        _ => return None,
    })
}

/// What [`judged`] found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Judged {
    /// Each DLL nothing gives the file that loads it, which Windows would refuse to start, or
    /// would stop when it first calls into a delay-loaded one.
    pub refused: Vec<Problem>,
    /// Each DLL a redistributable installs, when which the app ticks is not known, one an install
    /// script of the build may install, and one delay-loaded that nothing installs, which a file
    /// may be written to do without.
    pub unchecked: Vec<Problem>,
    /// The redistributables the app ticks that the build needs.
    pub ticked: BTreeSet<&'static str>,
}

/// What the Windows files `report` found load that neither Windows nor the build gives them.
///
/// Each is judged against `settings`, the app's as Steam shows them or a snapshot keeps them:
/// they say which depots install on Windows, so ship files there, and which redistributables the
/// app ticks. Without them, every depot counts as installing on Windows, and which are ticked is
/// not known.
#[must_use]
pub fn judged(report: &Report, settings: Option<&Block>) -> Judged {
    let depots = settings.and_then(|settings| settings.block("depots"));
    let on_windows = |depot_id: u32| {
        depots
            .and_then(|depots| depots.block(&depot_id.to_string()))
            .and_then(|depot| depot.block("config"))
            .and_then(|config| config.text("oslist"))
            .is_none_or(|systems| {
                systems.trim().is_empty()
                    || systems
                        .split(',')
                        .any(|system| system.trim().eq_ignore_ascii_case("windows"))
            })
    };
    let installing: Vec<_> = report
        .depots
        .iter()
        .filter(|depot| on_windows(depot.depot_id))
        .collect();
    let shipped: BTreeSet<String> = installing
        .iter()
        .flat_map(|depot| &depot.files)
        .filter_map(|file| file.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .collect();
    let ticked = settings.map(|_| depots.map(ticked).unwrap_or_default());
    let scripted = installing.iter().any(|depot| depot.install_scripts);
    let app = report.app_id.unwrap_or_default();
    let mut judged = Judged::default();
    for (file, imports) in installing.iter().flat_map(|depot| &depot.windows) {
        let mut seen = BTreeSet::new();
        let loaded = imports.loaded.iter().map(|dll| (dll, false));
        for (dll, delayed) in loaded.chain(imports.delayed.iter().map(|dll| (dll, true))) {
            let mut name = dll.to_ascii_lowercase();
            // Windows looks for a name without an extension as a DLL.
            if !name.contains('.') {
                name.push_str(".dll");
            }
            if WINDOWS_DLLS.contains(&name.as_str())
                || API_SETS.iter().any(|set| name.starts_with(set))
                || shipped.contains(&name)
                || !seen.insert(name.clone())
            {
                continue;
            }
            let loading = Loading {
                file,
                dll,
                name: &name,
                delayed,
            };
            judge(&mut judged, &loading, ticked.as_ref(), scripted, app);
        }
    }
    judged
}

/// The depots of app 228980 among `depots`: the redistributables the app ticks.
fn ticked(depots: &Block) -> BTreeSet<u32> {
    depots
        .pairs
        .iter()
        .filter(|pair| match &pair.value {
            Value::Block(depot) => depot.text("depotfromapp") == Some(REDISTRIBUTABLES),
            Value::Text(_) => false,
        })
        .filter_map(|pair| pair.key.parse().ok())
        .collect()
}

/// A DLL a Windows file loads that neither Windows nor the build gives it.
struct Loading<'names> {
    file: &'names Path,
    /// As the file spells it, and in lower case with its extension.
    dll: &'names str,
    name: &'names str,
    delayed: bool,
}

/// Judges what `loading` loads.
fn judge(
    judged: &mut Judged,
    loading: &Loading<'_>,
    ticked: Option<&BTreeSet<u32>>,
    scripted: bool,
    app: u32,
) {
    let &Loading {
        file,
        dll,
        name,
        delayed,
    } = loading;
    let verb = if delayed { "delay-loads" } else { "imports" };
    let Some(depots) = installed_by(name) else {
        let missing = format!(
            "{verb} {dll}, which neither Windows, the build's depots nor a Steamworks Common \
             Redistributable has"
        );
        if delayed {
            judged.unchecked.push(Problem::new(
                file,
                format!(
                    "{missing}; the file starts without it, and works only where it finds it or \
                     does without it"
                ),
            ));
        } else if scripted {
            judged.unchecked.push(Problem::new(
                file,
                format!("{missing}, unless an install script of the build installs it"),
            ));
        } else {
            judged.refused.push(Problem::new(
                file,
                format!("{missing}; ship it in a depot, or link its library in statically"),
            ));
        }
        return;
    };
    let names = named(depots);
    match ticked {
        Some(ticked) => {
            let found: Vec<&'static str> = NAMES
                .iter()
                .filter(|(depot, _)| depots.contains(depot) && ticked.contains(depot))
                .map(|pair| pair.1)
                .collect();
            if found.is_empty() {
                judged.refused.push(Problem::new(
                    file,
                    format!(
                        "{verb} {dll}, which {names} installs, and app {app} ticks none in \
                         Steamworks; tick one under Installation, Redistributables, ship the DLL \
                         in a depot, or link its library in statically"
                    ),
                ));
            } else {
                judged.ticked.extend(found);
            }
        }
        None => judged.unchecked.push(Problem::new(
            file,
            format!("{verb} {dll}, which {names} installs"),
        )),
    }
}

/// `depots` by their names, the last two joined by "or".
fn named(depots: &[u32]) -> String {
    let names: Vec<&str> = NAMES
        .iter()
        .filter(|(depot, _)| depots.contains(depot))
        .map(|&(_, name)| name)
        .collect();
    match names.as_slice() {
        [rest @ .., last] if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => names.concat(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by(dll: &str) -> Option<&'static [u32]> {
        installed_by(dll)
    }

    #[test]
    fn each_visual_cpp_runtime_is_installed_by_its_own_redistributable() {
        assert_eq!(by("msvcr80.dll"), Some(&[228_981][..]));
        assert_eq!(by("msvcp90.dll"), Some(&[228_982][..]));
        assert_eq!(by("msvcr100.dll"), Some(&[228_983][..]));
        assert_eq!(by("vccorlib110.dll"), Some(&[228_984][..]));
        assert_eq!(by("msvcp120.dll"), Some(&[228_985][..]));
    }

    #[test]
    fn a_dll_a_later_visual_cpp_added_is_installed_only_from_that_one_on() {
        assert_eq!(
            by("vcruntime140.dll"),
            Some(&[228_986, 228_987, 228_988, 228_989][..])
        );
        assert_eq!(by("msvcp140_2.dll"), Some(&[228_987, 228_988, 228_989][..]));
        assert_eq!(by("vcruntime140_1.dll"), Some(&[228_988, 228_989][..]));
        assert_eq!(by("msvcp140_atomic_wait.dll"), Some(&[228_989][..]));
    }

    #[test]
    fn directx_june_2010_installs_its_numbered_dlls_and_no_others() {
        for dll in [
            "d3dx9_24.dll",
            "d3dx9_43.dll",
            "d3dx10.dll",
            "d3dcompiler_43.dll",
            "xinput1_1.dll",
            "xinput1_3.dll",
            "xactengine2_10.dll",
        ] {
            assert_eq!(by(dll), Some(&[228_990][..]), "{dll}");
        }
        for dll in [
            "d3dx9_23.dll",
            "d3dx9_44.dll",
            "d3dcompiler_32.dll",
            "xinput1_4.dll",
            "xinput1_0.dll",
            "d3dx9_x.dll",
            "xinput1_3.lib",
        ] {
            assert_eq!(by(dll), None, "{dll}");
        }
    }

    #[test]
    fn openal_and_physx_are_installed_by_theirs() {
        assert_eq!(by("openal32.dll"), Some(&[229_020][..]));
        assert_eq!(
            by("physxloader.dll"),
            Some(&[229_030, 229_031, 229_032, 229_033][..])
        );
        assert_eq!(by("game.dll"), None);
    }

    #[test]
    fn several_redistributables_are_named_as_a_choice() {
        assert_eq!(named(&[228_989]), "VC 2022 Redist");
        assert_eq!(
            named(&[228_988, 228_989]),
            "VC 2019 Redist or VC 2022 Redist"
        );
        assert_eq!(
            named(&[228_987, 228_988, 228_989]),
            "VC 2017 Redist, VC 2019 Redist or VC 2022 Redist"
        );
    }
}
