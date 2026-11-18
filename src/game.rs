//! The game named by its folder: the one `.exe` in it that is no helper ([`exe_in`]).

use std::path::{Path, PathBuf};

/// The game in `folder`, named by its folder: the one `.exe` right in it, the helpers games ship
/// beside themselves aside ([`HELPER_EXES`]). None, or several, is said, naming what's there.
pub(crate) fn exe_in(folder: &Path) -> Result<PathBuf, String> {
    let mut exes: Vec<PathBuf> = std::fs::read_dir(folder)
        .map_err(|err| format!("{}: {err}", folder.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        })
        .collect();
    exes.sort();
    let names = |exes: &[&PathBuf]| -> String {
        let names: Vec<_> = exes
            .iter()
            .filter_map(|exe| exe.file_name())
            .map(|name| name.to_string_lossy())
            .collect();
        names.join(", ")
    };
    let games: Vec<&PathBuf> = exes.iter().filter(|exe| !is_helper(exe)).collect();
    match games.as_slice() {
        [game] => Ok(game.to_path_buf()),
        [] if exes.is_empty() => Err(format!("{} holds no .exe", folder.display())),
        [] => {
            let helpers: Vec<&PathBuf> = exes.iter().collect();
            Err(format!(
                "{} holds only helpers ({}) — name the .exe to run",
                folder.display(),
                names(&helpers)
            ))
        }
        several => Err(format!(
            "{} holds several .exe files — name the one to run: {}",
            folder.display(),
            names(several)
        )),
    }
}

/// Whether `exe` is one of the helpers games ship beside themselves, by its name's start.
fn is_helper(exe: &Path) -> bool {
    let name =
        exe.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    HELPER_EXES.iter().any(|helper| name.starts_with(helper))
}

/// The starts of the names of the `.exe` files games ship beside themselves that are no game:
/// Unity's crash handler and other crash reporters, uninstallers, runtime installers, and the
/// notification helper NW.js games (RPG Maker's among them) bring along.
const HELPER_EXES: &[&str] = &[
    "unitycrashhandler",
    "crashreport",
    "crashpad",
    "unins",
    "uninstall",
    "vc_redist",
    "vcredist",
    "dxsetup",
    "oalinst",
    "physx",
    "dotnet",
    "notification_helper",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proton::tests::scratch;

    /// A game named by its folder is the one `.exe` in it, Unity's crash handler, an uninstaller
    /// and NW.js's notification helper aside; a folder with several is answered with their names,
    /// as is one with none, or only helpers.
    #[test]
    fn a_game_named_by_its_folder_is_its_one_exe() {
        let base = scratch("exe_in");
        let folder = |name: &str, files: &[&str]| {
            let dir = base.join(name);
            std::fs::create_dir_all(dir.join("Data.exe")).unwrap(); // a folder named like one is none
            for file in files {
                std::fs::write(dir.join(file), "MZ").unwrap();
            }
            dir
        };
        let unity = folder(
            "unity",
            &["Some Game.exe", "UnityCrashHandler64.exe", "unins000.exe", "readme.txt"],
        );
        assert_eq!(exe_in(&unity), Ok(unity.join("Some Game.exe")));
        let nwjs = folder("nwjs", &["Game.exe", "notification_helper.exe"]);
        assert_eq!(exe_in(&nwjs), Ok(nwjs.join("Game.exe")));
        let upper = folder("upper", &["GAME.EXE"]);
        assert_eq!(exe_in(&upper), Ok(upper.join("GAME.EXE")));
        let two = folder("two", &["game.exe", "launcher.exe", "UnityCrashHandler64.exe"]);
        let several = exe_in(&two).unwrap_err();
        assert!(
            several.ends_with(
                "holds several .exe files — name the one to run: game.exe, launcher.exe"
            ),
            "{several}"
        );
        assert!(exe_in(&folder("none", &["readme.txt"])).unwrap_err().ends_with("holds no .exe"));
        let helpers =
            exe_in(&folder("helpers", &["unins000.exe", "vc_redist.x64.exe"])).unwrap_err();
        assert!(
            helpers.ends_with(
                "holds only helpers (unins000.exe, vc_redist.x64.exe) — name the .exe to run"
            ),
            "{helpers}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
