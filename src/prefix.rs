//! A game's own folder — its Wine prefix (in `pfx`) and the home the sandbox gives it — made for it
//! under [`PREFIXES_DIR`] and named for its folder ([`own_folder`]); and the prefixes other
//! launchers made for a game ([`made_elsewhere`]: Steam's, so far), which a run can go on from,
//! where they are or copied into the game's own folder ([`copy`]).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::steam;

/// Where each game's own folder is made, in the home: in `~/Games`, beside the user's other
/// games.
pub(crate) const PREFIXES_DIR: &str = "Games/manual_run_proton_prefixes";
/// The Wine prefix proper — the C: drive and the registry — in a game's own folder, as Proton
/// keeps it in the folder it's given.
pub(crate) const PFX_DIR: &str = "pfx";

/// The game's own folder, by default: in [`PREFIXES_DIR`] under `home`, named for the game's
/// folder `game_dir`, then a checksum of that folder's whole path, so that games in same-named
/// folders never share one.
pub(crate) fn own_folder(home: &Path, game_dir: &Path) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let name = game_dir
        .file_name()
        .map_or_else(|| "game".to_string(), |name| name.to_string_lossy().into_owned());
    let checksum = crc32fast::hash(game_dir.as_os_str().as_bytes());
    home.join(PREFIXES_DIR).join(format!("{name}-{checksum:08x}"))
}

/// A prefix another launcher made for a game: the launcher, its name for the game, and the
/// prefix's folder — shaped as the game's own folder is, with the Wine prefix in its `pfx`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PastPrefix {
    pub(crate) launcher: &'static str,
    pub(crate) game: String,
    pub(crate) folder: PathBuf,
}

impl PastPrefix {
    /// How a menu and a message name it: whose prefix, for which game, and where.
    pub(crate) fn described(&self) -> String {
        format!("{}'s prefix for {} ({})", self.launcher, self.game, self.folder.display())
    }
}

/// Every prefix another launcher made for the game at `exe`, as each of [`LAUNCHERS`] finds them
/// under `home`, in their order.
pub(crate) fn made_elsewhere(exe: &Path, home: &Path) -> Vec<PastPrefix> {
    LAUNCHERS
        .iter()
        .flat_map(|&(launcher, find)| {
            find(exe, home).into_iter().map(move |(game, folder)| PastPrefix {
                launcher,
                game,
                folder,
            })
        })
        .collect()
}

/// The names of the launchers whose prefixes are looked for, in order.
pub(crate) fn launchers() -> Vec<&'static str> {
    LAUNCHERS.iter().map(|(launcher, _)| *launcher).collect()
}

/// What finds a launcher's prefixes for an .exe under a home: for each, the launcher's name for
/// the game, and the prefix's folder.
type FindPrefixes = fn(&Path, &Path) -> Vec<(String, PathBuf)>;

/// The launchers whose prefixes a run can go on from: each one's name, and what finds its
/// prefixes. A new launcher is a row.
const LAUNCHERS: &[(&str, FindPrefixes)] = &[("Steam", steam_prefixes_of)];

/// The prefixes Steam made for the game at `exe`, under `home`: for each Steam entry that is the
/// game's — a non-Steam game added with that .exe, or an app installed in a folder holding it —
/// the prefix Proton made under its id, in each library where there is one (a folder holding the
/// prefix proper, `pfx`: Steam also makes them empty), each once.
fn steam_prefixes_of(exe: &Path, home: &Path) -> Vec<(String, PathBuf)> {
    let known = steam::Known::gather(home);
    let exe = crate::canonical(exe);
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for game in known.games.iter().filter(|game| game.runs(&exe)) {
        let name = match game.title.trim() {
            "" => game.id.to_string(),
            title => title.to_string(),
        };
        for folder in known.prefixes_of(game.id) {
            if folder.join(PFX_DIR).is_dir() && !found.iter().any(|(_, known)| *known == folder) {
                found.push((name.clone(), folder));
            }
        }
    }
    found
}

/// `from`, a prefix, copied whole to `to` — links as links, modes and times kept, the data shared
/// where the filesystem can (`cp -a --reflink=auto`) — by way of a folder beside `to`, renamed to
/// it once the copy is whole, so that a copy cut short is never taken for a prefix. Why not, where
/// it fails; what the copy made is gone then.
pub(crate) fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let (Some(parent), Some(name)) = (to.parent(), to.file_name()) else {
        return Err(format!("{} names no folder to copy into", to.display()));
    };
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("couldn't make {}: {err}", parent.display()))?;
    let copying =
        parent.join(format!(".{}.copying-{}", name.to_string_lossy(), std::process::id()));
    let copied =
        Command::new("cp").args(["-a", "--reflink=auto", "--"]).arg(from).arg(&copying).status();
    let done = match copied {
        Ok(status) if status.success() => std::fs::rename(&copying, to)
            .map_err(|err| format!("couldn't move the copy to {}: {err}", to.display())),
        Ok(status) => Err(format!("copying {} failed ({status})", from.display())),
        Err(err) => Err(format!("couldn't run cp to copy {}: {err}", from.display())),
    };
    if done.is_err() {
        let _ = std::fs::remove_dir_all(&copying);
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proton::tests::scratch;
    use crate::steam::tests::shortcuts_vdf;

    /// A game's own folder is named for its folder, with a checksum of where that is: the same game
    /// always gets the same one, and same-named games in different places get their own.
    #[test]
    fn each_game_gets_a_folder_of_its_own_named_for_it() {
        let home = Path::new("/home/u");
        let one = own_folder(home, Path::new("/games/Hollow Knight"));
        let name = one.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(one.parent(), Some(Path::new("/home/u/Games/manual_run_proton_prefixes")));
        assert!(
            name.starts_with("Hollow Knight-") && name.len() == "Hollow Knight-".len() + 8,
            "{name}"
        );
        assert_eq!(one, own_folder(home, Path::new("/games/Hollow Knight")));
        assert_ne!(one, own_folder(home, Path::new("/other/Hollow Knight")));
        assert_eq!(
            name, "Hollow Knight-9ac7b0ec",
            "the checksum is CRC-32's: existing folders keep their names"
        );
    }

    /// The prefixes Steam made for an .exe are found by the Steam entries that are its game — a
    /// non-Steam game added with that .exe, even through a link, and an app installed around it —
    /// each with the prefix Proton made under its id, in whichever library it is. An entry whose
    /// prefix Steam made but never filled, and another game's, are no match.
    #[test]
    fn the_prefixes_steam_made_for_an_exe_are_found_by_its_entries() {
        let home = scratch("past_prefixes");
        let steam = home.join(".local/share/Steam");
        let disk = home.join("disk");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        let listed = format!(
            "\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            disk.display()
        );
        std::fs::write(steam.join("steamapps/libraryfolders.vdf"), listed).unwrap();
        let portal = disk.join("steamapps/common/Portal 2");
        let (ice, other) = (home.join("games/ice"), home.join("games/other"));
        for folder in [&portal.join("bin"), &ice, &other] {
            std::fs::create_dir_all(folder).unwrap();
        }
        for exe in [portal.join("bin/portal2.exe"), ice.join("Game.exe"), other.join("other.exe")] {
            std::fs::write(exe, "MZ").unwrap();
        }
        let manifest = "\"AppState\"\n{\n\t\"appid\"\t\t\"620\"\n\t\"name\"\t\t\"Portal 2\"\n\t\"installdir\"\t\t\"Portal 2\"\n}\n";
        std::fs::write(disk.join("steamapps/appmanifest_620.acf"), manifest).unwrap();
        std::os::unix::fs::symlink(home.join("games"), home.join("linked")).unwrap();
        let quoted = |exe: PathBuf| format!("\"{}\"", exe.display());
        let (through_link, ice_exe, other_exe) = (
            quoted(home.join("linked/ice/Game.exe")),
            quoted(ice.join("Game.exe")),
            quoted(other.join("other.exe")),
        );
        let added = [
            (Some(3_640_112_877), "itchio-Cirno-ice-shave", through_link.as_str()),
            (Some(3_000_000_001), "Ice, again", ice_exe.as_str()),
            (Some(3_000_000_002), "Ice, never run", ice_exe.as_str()),
            (Some(3_000_000_003), "Other", other_exe.as_str()),
        ];
        std::fs::create_dir_all(steam.join("userdata/1/config")).unwrap();
        std::fs::write(
            steam.join("userdata/1/config/shortcuts.vdf"),
            shortcuts_vdf(&added, ["AppName", "Exe"]),
        )
        .unwrap();
        let compat = |library: &Path, id: &str| library.join("steamapps/compatdata").join(id);
        for (library, id) in
            [(&steam, "3640112877"), (&steam, "3000000001"), (&steam, "3000000003"), (&disk, "620")]
        {
            std::fs::create_dir_all(compat(library, id).join(PFX_DIR)).unwrap();
        }
        std::fs::create_dir_all(compat(&steam, "3000000002")).unwrap(); // made, never run

        assert_eq!(
            steam_prefixes_of(&ice.join("Game.exe"), &home),
            [
                ("itchio-Cirno-ice-shave".to_string(), compat(&steam, "3640112877")),
                ("Ice, again".to_string(), compat(&steam, "3000000001"))
            ]
        );
        assert_eq!(
            steam_prefixes_of(&portal.join("bin/portal2.exe"), &home),
            [("Portal 2".to_string(), compat(&disk, "620"))]
        );
        assert!(steam_prefixes_of(&home.join("games/unknown/game.exe"), &home).is_empty());
        let past = made_elsewhere(&other.join("other.exe"), &home);
        assert_eq!(
            past,
            [PastPrefix {
                launcher: "Steam",
                game: "Other".into(),
                folder: compat(&steam, "3000000003")
            }]
        );
        assert_eq!(
            past[0].described(),
            format!("Steam's prefix for Other ({})", compat(&steam, "3000000003").display())
        );
        assert_eq!(launchers(), ["Steam"]);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A prefix is copied whole — its links as links, relative and absolute, its files' modes kept
    /// — and its source stays; a copy that fails leaves nothing behind, where it was to go or
    /// beside it.
    #[test]
    fn a_prefix_is_copied_whole_or_not_at_all() {
        use std::os::unix::fs::PermissionsExt;
        let base = scratch("copy_prefix");
        let from = base.join("compatdata/42");
        std::fs::create_dir_all(from.join("pfx/drive_c/users/steamuser")).unwrap();
        std::fs::create_dir_all(from.join("pfx/dosdevices")).unwrap();
        std::fs::write(from.join("pfx/user.reg"), "WINE REGISTRY").unwrap();
        std::fs::set_permissions(from.join("pfx/user.reg"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
        std::os::unix::fs::symlink("../drive_c", from.join("pfx/dosdevices/c:")).unwrap();
        std::os::unix::fs::symlink("/", from.join("pfx/dosdevices/z:")).unwrap();

        let to = base.join("prefixes/game-1");
        assert_eq!(copy(&from, &to), Ok(()));
        assert_eq!(std::fs::read_to_string(to.join("pfx/user.reg")).unwrap(), "WINE REGISTRY");
        assert_eq!(
            std::fs::metadata(to.join("pfx/user.reg")).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::read_link(to.join("pfx/dosdevices/c:")).unwrap(),
            Path::new("../drive_c")
        );
        assert_eq!(std::fs::read_link(to.join("pfx/dosdevices/z:")).unwrap(), Path::new("/"));
        assert!(to.join("pfx/drive_c/users/steamuser").is_dir());
        assert!(from.join("pfx/user.reg").is_file(), "the source stays");
        let names = |dir: &Path| {
            std::fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&base.join("prefixes")), ["game-1"], "nothing beside it");

        let failed = copy(&base.join("no-such-prefix"), &base.join("prefixes/game-2"));
        assert!(failed.is_err(), "{failed:?}");
        assert_eq!(
            names(&base.join("prefixes")),
            ["game-1"],
            "nothing made, where it was to go or beside it"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
