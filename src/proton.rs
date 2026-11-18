//! Proton, the layer a game runs through: every copy this computer has ([`installed`]) — wherever
//! Steam or a package put one, and one the program running this keeps itself — each read off its
//! own folder: its name and build time from its `version` file, and whether it can run a game on
//! Wayland itself; and which of them a run takes ([`choose`]).
//!
//! A Proton is a folder holding a `proton` script and a `version` file. Steam keeps Valve's in its
//! libraries' `steamapps/common`, and other builds (GE-Proton, proton-cachyos, …) in a
//! `compatibilitytools.d`: its own, and the system-wide ones packages install into.

use std::path::{Path, PathBuf};

use crate::steam;

/// One copy of Proton.
#[derive(Debug, Clone, PartialEq)]
pub struct Proton {
    /// Its folder, holding the `proton` script.
    pub dir: PathBuf,
    /// Its name, as its `version` file gives it (`cachyos-11.0-20261015-slr`,
    /// `experimental-11.0-20261112-x86_64`, `GE-Proton11-12`).
    pub name: String,
    /// When it was built, in Unix seconds, from the same file: what "newest" goes by.
    pub built: u64,
    /// Whether it can run a game on Wayland itself: it ships Wine's Wayland driver, and its script
    /// switches it on by [`WAYLAND_SWITCH`].
    pub wayland: bool,
    /// Whether it is the one the program running this keeps itself.
    pub own: bool,
}

/// Every Proton under `home` and the system's, newest first, each once — and `own`, the one the
/// program running this keeps itself, where there is one there.
pub fn installed(home: &Path, own: Option<&Path>) -> Vec<Proton> {
    let mut protons: Vec<Proton> = own.and_then(|dir| read(dir, true)).into_iter().collect();
    protons.extend(found_in(&search_dirs(home, SYSTEM_TOOL_DIRS)));
    newest_first(protons)
}

/// The Proton in `dir`, wherever it is: one named by its folder, which it holds whole — links
/// followed, and from `/`, so that it is found the same from any folder (the sandbox runs it from
/// the game's). It is the program's own where it is `own`, whichever way it was reached.
pub fn at(dir: &Path, own: Option<&Path>) -> Option<Proton> {
    let dir = std::fs::canonicalize(dir).ok()?;
    let own = own.and_then(|own| std::fs::canonicalize(own).ok());
    read(&dir, own.as_ref() == Some(&dir))
}

/// The Proton a run takes, whether it runs the game on Wayland, and what's worth saying about the
/// pick.
#[derive(Debug)]
pub struct Choice<'a> {
    pub proton: &'a Proton,
    pub wayland: bool,
    pub note: Option<String>,
}

/// Which Proton runs the game. One `asked` for: a Proton's folder, or a name among `protons` (or
/// part of one, the newest such). Else the program's own where `own_first`, else the newest of
/// them all — and, for a game to run on Wayland, the newest with a Wayland driver, unless none has
/// one. It runs on Wayland only where asked to and able to.
pub fn choose<'a>(
    protons: &'a [Proton],
    asked: Option<&str>,
    wayland: bool,
    own_first: bool,
) -> Result<Choice<'a>, String> {
    if let Some(asked) = asked {
        let folder = std::fs::canonicalize(asked).ok();
        let lower = asked.to_lowercase();
        let named = protons.iter().find(|proton| {
            folder.as_ref().is_some_and(|folder| {
                std::fs::canonicalize(&proton.dir).is_ok_and(|dir| dir == *folder)
            }) || proton.name.to_lowercase().contains(&lower)
        });
        let Some(proton) = named else {
            return Err(format!(
                "{asked} is no Proton's folder, and no Proton here is called that"
            ));
        };
        let note = (wayland && !proton.wayland)
            .then(|| format!("{} has no Wayland driver, so the game runs on X11", proton.name));
        return Ok(Choice { proton, wayland: wayland && proton.wayland, note });
    }
    let candidates: Vec<&Proton> = if own_first && protons.iter().any(|proton| proton.own) {
        protons.iter().filter(|proton| proton.own).collect()
    } else {
        protons.iter().collect()
    };
    let Some(&newest) = candidates.first() else {
        return Err("there's no Proton on this computer".to_string());
    };
    if !wayland {
        return Ok(Choice { proton: newest, wayland: false, note: None });
    }
    Ok(match candidates.iter().copied().find(|proton| proton.wayland) {
        Some(proton) if proton.dir == newest.dir => Choice { proton, wayland: true, note: None },
        Some(proton) => Choice {
            proton,
            wayland: true,
            note: Some(format!(
                "the newest Proton, {}, has no Wayland driver (--xwayland runs the game on it)",
                newest.name
            )),
        },
        None => Choice {
            proton: newest,
            wayland: false,
            note: Some("no Proton here has a Wayland driver, so the game runs on X11".to_string()),
        },
    })
}

/// The system-wide `compatibilitytools.d` folders packages install Protons into, which Steam
/// reads beside its own.
const SYSTEM_TOOL_DIRS: &[&str] =
    &["/usr/share/steam/compatibilitytools.d", "/usr/local/share/steam/compatibilitytools.d"];

/// The folders whose sub-folders may be Protons: each Steam's `compatibilitytools.d`, the
/// `steamapps/common` of each of its [`steam::libraries`], and the system-wide `system_dirs`.
fn search_dirs(home: &Path, system_dirs: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for root in steam::roots(home) {
        dirs.push(root.join("compatibilitytools.d"));
        dirs.extend(
            steam::libraries(&root).into_iter().map(|library| library.join("steamapps/common")),
        );
    }
    dirs.extend(system_dirs.iter().map(PathBuf::from));
    dirs
}

/// The Protons among the sub-folders of `dirs`, each once however many ways it was reached (a
/// library listed twice, `~/.steam/root` beside `~/.local/share/Steam`).
fn found_in(dirs: &[PathBuf]) -> Vec<Proton> {
    let mut seen = std::collections::HashSet::new();
    dirs.iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| read(&entry.path(), false))
        .filter(|proton| seen.insert(crate::canonical(&proton.dir)))
        .collect()
}

/// `protons` ordered newest build first, a tie by name.
fn newest_first(mut protons: Vec<Proton>) -> Vec<Proton> {
    protons.sort_by(|a, b| b.built.cmp(&a.built).then_with(|| a.name.cmp(&b.name)));
    protons
}

/// The Proton in `dir`, if it holds one: a `proton` script, and a `version` file reading
/// `<build time> <name>`.
fn read(dir: &Path, own: bool) -> Option<Proton> {
    if !dir.join("proton").is_file() {
        return None;
    }
    let version = std::fs::read_to_string(dir.join("version")).ok()?;
    let (built, name) = version.lines().next()?.trim().split_once(' ')?;
    Some(Proton {
        dir: dir.to_path_buf(),
        name: name.trim().to_string(),
        built: built.parse().ok()?,
        wayland: wayland_capable(dir),
        own,
    })
}

/// Whether the Proton in `dir` can run a game on Wayland itself: it ships Wine's Wayland driver,
/// and its script reads the switch that turns it on.
fn wayland_capable(dir: &Path) -> bool {
    let driver = WAYLAND_DRIVER.iter().any(|path| dir.join(path).is_file());
    driver
        && std::fs::read_to_string(dir.join("proton"))
            .is_ok_and(|script| script.contains(WAYLAND_SWITCH))
}

/// Where a Proton keeps Wine's Wayland driver, by the layouts Protons have had.
const WAYLAND_DRIVER: &[&str] =
    &["files/lib/wine/x86_64-unix/winewayland.so", "files/lib64/wine/x86_64-unix/winewayland.so"];

/// The environment variable that switches a Proton's Wayland driver on.
pub const WAYLAND_SWITCH: &str = "PROTON_ENABLE_WAYLAND";

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A scratch directory for one test, gone afterwards.
    pub(crate) fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("proton_sandboxed_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stand-in Proton in `dir`: a `proton` script (`body` its text), a `version` file, and Wine's
    /// Wayland driver when `wayland` says so.
    pub(crate) fn fake_proton(dir: &Path, version: &str, wayland: bool, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let script = dir.join("proton");
        std::fs::write(&script, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("version"), format!("{version}\n")).unwrap();
        if wayland {
            let driver = dir.join(WAYLAND_DRIVER[0]);
            std::fs::create_dir_all(driver.parent().unwrap()).unwrap();
            std::fs::write(driver, "").unwrap();
        }
    }

    /// A stand-in for an installed Proton: just what choosing reads.
    fn proton(name: &str, built: u64, wayland: bool, own: bool) -> Proton {
        Proton {
            dir: PathBuf::from(format!("/protons/{name}")),
            name: name.to_string(),
            built,
            wayland,
            own,
        }
    }

    /// A Proton is read off its own files: the name and build time from `version`, and Wayland
    /// only where both the driver and the switch are there. A folder missing either file is none.
    #[test]
    fn a_proton_is_read_off_its_version_file_and_its_driver() {
        let base = scratch("read");
        fake_proton(
            &base.join("cachy"),
            "1792530497 cachyos-11.0-20261015-slr",
            true,
            "x = 'PROTON_ENABLE_WAYLAND'\n",
        );
        fake_proton(
            &base.join("valve"),
            "1794497090 experimental-11.0-20261112-x86_64",
            false,
            "# no switch\n",
        );
        fake_proton(&base.join("driver-no-switch"), "1 odd", true, "# no switch\n");
        let cachy = read(&base.join("cachy"), false).unwrap();
        assert_eq!(
            (cachy.name.as_str(), cachy.built, cachy.wayland),
            ("cachyos-11.0-20261015-slr", 1792530497, true)
        );
        assert!(!read(&base.join("valve"), false).unwrap().wayland);
        assert!(
            !read(&base.join("driver-no-switch"), false).unwrap().wayland,
            "a driver nothing switches on is no use"
        );
        assert!(read(&base.join("valve"), true).unwrap().own);

        std::fs::create_dir_all(base.join("no-version")).unwrap();
        std::fs::write(base.join("no-version/proton"), "").unwrap();
        std::fs::create_dir_all(base.join("no-script")).unwrap();
        std::fs::write(base.join("no-script/version"), "1 x\n").unwrap();
        std::fs::create_dir_all(base.join("bad-version")).unwrap();
        std::fs::write(base.join("bad-version/proton"), "").unwrap();
        std::fs::write(base.join("bad-version/version"), "soon\n").unwrap();
        for not_one in ["no-version", "no-script", "bad-version", "nowhere"] {
            assert_eq!(read(&base.join(not_one), false), None, "{not_one}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The places searched: each Steam's compatibility tools and libraries (its own and the ones
    /// `libraryfolders.vdf` adds), then the system's; every Proton found once, newest first, and
    /// anything that is no Proton (a Steam runtime, a game) passed over. The program's own is
    /// among them, marked so, wherever it was reached from.
    #[test]
    fn protons_are_found_in_steams_libraries_and_tool_folders_newest_first() {
        let home = scratch("search");
        let steam = home.join(".local/share/Steam");
        let other_library = home.join("games-disk");
        let system = home.join("usr-share-steam-compatibilitytools.d");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                steam.display(),
                other_library.display()
            ),
        )
        .unwrap();
        fake_proton(
            &steam.join("steamapps/common/Proton - Experimental"),
            "300 experimental-11.0",
            false,
            "",
        );
        fake_proton(
            &other_library.join("steamapps/common/Proton 9.0 (Beta)"),
            "100 proton-9.0",
            false,
            "",
        );
        fake_proton(
            &steam.join("compatibilitytools.d/GE-Proton11-12"),
            "200 GE-Proton11-12",
            true,
            "PROTON_ENABLE_WAYLAND",
        );
        fake_proton(
            &system.join("proton-cachyos-slr"),
            "250 cachyos-11.0",
            true,
            "PROTON_ENABLE_WAYLAND",
        );
        std::fs::create_dir_all(steam.join("steamapps/common/SteamLinuxRuntime_4")).unwrap();
        std::fs::create_dir_all(steam.join("steamapps/common/Some Game")).unwrap();
        std::os::unix::fs::symlink(&steam, home.join("steam-again")).unwrap();

        let system_dir = system.to_str().unwrap();
        let mut dirs = search_dirs(&home, &[system_dir]);
        dirs.push(home.join("steam-again/compatibilitytools.d")); // the same tools, reached again
        let names: Vec<String> =
            newest_first(found_in(&dirs)).into_iter().map(|proton| proton.name).collect();
        assert_eq!(names, ["experimental-11.0", "cachyos-11.0", "GE-Proton11-12", "proton-9.0"]);

        let own = home.join("own-proton");
        fake_proton(&own, "50 own-build", false, "");
        let installed = installed(&home, Some(&own));
        assert!(
            installed.iter().any(|proton| proton.name == "own-build" && proton.own),
            "{installed:?}"
        );
        assert!(at(&own, Some(&own)).unwrap().own && !at(&own, None).unwrap().own);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The newest runs the game; on Wayland, the newest with a Wayland driver, saying the newest
    /// has none; where none has one, the newest on X11. With the program's own preferred, its own,
    /// however old. One asked for by name (or part of one) is taken as asked, on X11 when it has
    /// no Wayland driver; a name nothing has is refused.
    #[test]
    fn the_newest_proton_is_chosen_with_a_wayland_driver_where_wayland_is_wanted() {
        let protons = [
            proton("experimental-11.0", 300, false, false),
            proton("cachyos-11.0", 250, true, false),
            proton("GE-Proton9", 100, true, true),
        ];
        let pick = |asked, wayland, own_first| {
            let choice = choose(&protons, asked, wayland, own_first).unwrap();
            (choice.proton.name.clone(), choice.wayland, choice.note)
        };
        assert_eq!(pick(None, false, false), ("experimental-11.0".into(), false, None));
        let (name, wayland, note) = pick(None, true, false);
        assert_eq!((name.as_str(), wayland), ("cachyos-11.0", true));
        assert!(
            note.unwrap().contains("the newest Proton, experimental-11.0, has no Wayland driver")
        );
        assert_eq!(
            pick(None, true, true),
            ("GE-Proton9".into(), true, None),
            "the program's own first, where it says so"
        );
        assert_eq!(pick(Some("Experimental"), true, false).0, "experimental-11.0");
        assert!(!pick(Some("experimental"), true, false).1, "no driver, so X11");
        assert_eq!(pick(Some("ge-proton"), true, false), ("GE-Proton9".into(), true, None));
        assert!(choose(&protons, Some("proton-tkg"), true, false).is_err());

        let x11_only =
            [proton("experimental-11.0", 300, false, false), proton("proton-9", 100, false, false)];
        let choice = choose(&x11_only, None, true, false).unwrap();
        assert_eq!((choice.proton.name.as_str(), choice.wayland), ("experimental-11.0", false));
        assert!(choice.note.unwrap().contains("no Proton here has a Wayland driver"));
        assert!(choose(&[], None, true, false).is_err());
    }

    /// A Proton is also asked for by its folder, found by where it is, even through a link to it;
    /// one read off its folder holds it whole, however it was named — through a link, or from
    /// another folder — and is the program's own where that folder is.
    #[test]
    fn a_proton_is_also_asked_for_by_its_folder() {
        let base = scratch("asked_folder");
        fake_proton(&base.join("custom"), "5 custom-build", false, "");
        std::os::unix::fs::symlink(base.join("custom"), base.join("link")).unwrap();
        let whole = std::fs::canonicalize(base.join("custom")).unwrap();
        let up: PathBuf =
            std::env::current_dir().unwrap().components().skip(1).map(|_| "..").collect();
        let relative = up.join(whole.strip_prefix("/").unwrap());
        for named in [base.join("link"), relative] {
            let found = at(&named, None).unwrap();
            assert_eq!((found.dir, found.own), (whole.clone(), false), "{named:?}");
        }
        assert!(at(&base.join("link"), Some(&base.join("custom"))).unwrap().own);
        assert_eq!(at(&base.join("nothing-here"), None), None);
        let protons = [Proton {
            dir: base.join("custom"),
            name: "custom-build".into(),
            built: 5,
            wayland: false,
            own: false,
        }];
        for asked in [base.join("custom"), base.join("link")] {
            let choice = choose(&protons, Some(asked.to_str().unwrap()), false, false).unwrap();
            assert_eq!(choice.proton.dir, base.join("custom"));
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
