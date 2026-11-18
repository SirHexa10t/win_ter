//! What Steam keeps, read off this computer: where it keeps itself ([`roots`]) and its libraries
//! ([`libraries`]), and the games it knows ([`Known`]).
//!
//! Steam keeps a game in one of its libraries — its own folder, and those its `libraryfolders.vdf`
//! lists: the install in `steamapps/common`, in the folder the game's manifest names
//! (`steamapps/appmanifest_<id>.acf`), and its Proton prefix in `steamapps/compatdata/<id>`. In
//! Steam's own folder, `userdata` holds a folder per account: each game's Steam Cloud files
//! (`<account>/<id>`), and the non-Steam games the account added (`<account>/config/shortcuts.vdf`,
//! binary KeyValues), each under the id Steam gave it, kept in that file — nothing rebuilds it from
//! the game's path or name.

use std::collections::HashSet;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Where Steam keeps itself under `home`: each of [`STEAM_ROOTS`] that is there.
pub fn roots(home: &Path) -> Vec<PathBuf> {
    STEAM_ROOTS.iter().map(|root| home.join(root)).filter(|root| root.is_dir()).collect()
}

/// The libraries of the Steam at `root`: its own folder, then those its `libraryfolders.vdf` lists
/// (its own among them, as Steam writes the file).
pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let listed =
        std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    std::iter::once(root.to_path_buf()).chain(library_paths(&listed)).collect()
}

/// Where Steam keeps itself under a home: the native install, the Flatpak and the Snap.
const STEAM_ROOTS: &[&str] = &[
    ".local/share/Steam",
    ".var/app/com.valvesoftware.Steam/.local/share/Steam",
    "snap/steam/common/.local/share/Steam",
];

/// The libraries a `libraryfolders.vdf` lists: the value of each `"path"` key, its doubled
/// backslashes undone (Valve's KeyValues format escapes them).
fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| line.trim().strip_prefix("\"path\""))
        .filter_map(|value| {
            let path = value.trim().strip_prefix('"')?.strip_suffix('"')?;
            Some(PathBuf::from(path.replace("\\\\", "\\")))
        })
        .collect()
}

/// What this computer knows of Steam games: every Steam library (each Steam's own folder, and
/// those it lists), every account's folder in each Steam's `userdata`, and the games — Steam's,
/// from the manifests in its libraries, and the non-Steam ones the accounts added.
#[derive(Debug, Default)]
pub struct Known {
    pub libraries: Vec<PathBuf>,
    pub accounts: Vec<PathBuf>,
    pub games: Vec<Game>,
}

/// A game Steam knows: its app id, its title there, the folder it's installed in (an app's,
/// under `steamapps/common`; a non-Steam game's, its .exe's), and a non-Steam game's .exe.
#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub id: u32,
    pub title: String,
    pub folder: Option<PathBuf>,
    pub exe: Option<PathBuf>,
}

impl Game {
    /// Whether the .exe at `exe` (a canonical path) runs this game: it's a non-Steam game's own
    /// .exe, or one in an app's install folder.
    pub fn runs(&self, exe: &Path) -> bool {
        match (&self.exe, &self.folder) {
            (Some(own), _) => crate::canonical(own) == exe,
            (None, Some(folder)) => exe.starts_with(crate::canonical(folder)),
            (None, None) => false,
        }
    }
}

impl Known {
    /// What the Steams under `home` keep: their libraries, each once however it's reached (a
    /// Steam's own folder is listed in its `libraryfolders.vdf` too); the accounts in their
    /// `userdata`; the apps the libraries' manifests name, and the non-Steam games in each
    /// account's `shortcuts.vdf`.
    pub fn gather(home: &Path) -> Self {
        let mut known = Known::default();
        let mut seen = HashSet::new();
        for root in roots(home) {
            for library in libraries(&root) {
                if seen.insert(crate::canonical(&library)) {
                    known.libraries.push(library);
                }
            }
            known.accounts.extend(accounts(&root.join(USERDATA_DIR)));
        }
        for library in &known.libraries {
            known.games.extend(manifests(library));
        }
        for account in &known.accounts {
            let shortcuts = std::fs::read(account.join(SHORTCUTS_FILE)).unwrap_or_default();
            known.games.extend(shortcuts_in(&shortcuts));
        }
        known
    }

    /// The folders the game with the id `id` is installed in that are there, in path order.
    pub fn installs_of(&self, id: u32) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = self
            .games
            .iter()
            .filter(|game| game.id == id)
            .filter_map(|game| game.folder.clone())
            .filter(|folder| folder.is_dir())
            .collect();
        folders.sort();
        folders.dedup();
        folders
    }

    /// The title of the game with the id `id`, as Steam shows it.
    pub fn title_of(&self, id: u32) -> String {
        self.games
            .iter()
            .find(|game| game.id == id)
            .map(|game| game.title.trim().to_string())
            .unwrap_or_default()
    }

    /// The Proton prefixes Steam made under the id `id` — its folder in each library's
    /// `steamapps/compatdata` — those that are there.
    pub fn prefixes_of(&self, id: u32) -> Vec<PathBuf> {
        self.libraries
            .iter()
            .map(|library| library.join(COMPATDATA_DIR).join(id.to_string()))
            .filter(|folder| folder.is_dir())
            .collect()
    }

    /// The Steam Cloud folders of the game with the id `id` — its folder in each account's — those
    /// that are there.
    pub fn cloud_folders_of(&self, id: u32) -> Vec<PathBuf> {
        self.accounts
            .iter()
            .map(|account| account.join(id.to_string()))
            .filter(|folder| folder.is_dir())
            .collect()
    }
}

/// The accounts' folders in a Steam's `userdata`, each named for its account's id, in order.
fn accounts(userdata: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(userdata) else {
        return Vec::new();
    };
    let mut accounts: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().as_bytes().iter().all(u8::is_ascii_digit))
        .map(|entry| entry.path())
        .filter(|folder| folder.is_dir())
        .collect();
    accounts.sort();
    accounts
}

/// The apps installed in the Steam library at `library`, by their manifests
/// (`steamapps/appmanifest_<id>.acf`), in id order: each one's id, title and install folder. A
/// manifest that doesn't read, or names no id, is no app.
fn manifests(library: &Path) -> Vec<Game> {
    let steamapps = library.join(STEAMAPPS_DIR);
    let Ok(entries) = std::fs::read_dir(&steamapps) else {
        return Vec::new();
    };
    let is_manifest =
        |name: &str| name.starts_with(MANIFEST_PREFIX) && name.ends_with(MANIFEST_SUFFIX);
    let mut games: Vec<Game> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_str().is_some_and(is_manifest))
        .filter_map(|entry| manifest_game(&std::fs::read_to_string(entry.path()).ok()?, &steamapps))
        .collect();
    games.sort_by_key(|game| game.id);
    games
}

/// The app an app manifest names, installed under `steamapps`: its `appid`, which it must have,
/// its `name`, and its `installdir`, its folder in `steamapps/common`.
fn manifest_game(acf: &str, steamapps: &Path) -> Option<Game> {
    let pairs = vdf_pairs(acf);
    let value = |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    };
    let id: u32 = value("appid")?.trim().parse().ok()?;
    let folder = value("installdir")
        .filter(|dir| !dir.is_empty())
        .map(|dir| steamapps.join(COMMON_DIR).join(dir));
    Some(Game { id, title: value("name").unwrap_or_default().to_string(), folder, exe: None })
}

/// The `"key" "value"` pairs of Valve's text KeyValues (an app manifest), at any depth, in order:
/// a line holding two quoted strings is a pair, as Steam writes them, each string's escapes
/// (`\"`, `\\`, `\n`, `\t`) undone.
fn vdf_pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| match <[String; 2]>::try_from(quoted(line)) {
            Ok([key, value]) => Some((key, value)),
            Err(_) => None,
        })
        .collect()
}

/// The quoted strings on a line of Valve's text KeyValues, their escapes undone; one with no
/// closing quote is left out.
fn quoted(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut chars = line.chars();
    while chars.any(|c| c == '"') {
        let mut text = String::new();
        loop {
            match chars.next() {
                Some('"') => break,
                Some('\\') => match chars.next() {
                    Some('n') => text.push('\n'),
                    Some('t') => text.push('\t'),
                    Some(escaped) => text.push(escaped),
                    None => return found,
                },
                Some(c) => text.push(c),
                None => return found,
            }
        }
        found.push(text);
    }
    found
}

/// The non-Steam games an account added to Steam, as its `shortcuts.vdf` keeps them (Valve's
/// binary KeyValues: a `shortcuts` map holding a map per game): each one's id, its title, its .exe
/// (quoted, as Steam writes it, or not) and that .exe's folder — none for a command that is no
/// path. One with no id is passed over; a file that doesn't read gives none.
fn shortcuts_in(vdf: &[u8]) -> Vec<Game> {
    let mut at = 0;
    let Some(root) = binary_map(vdf, &mut at) else {
        return Vec::new();
    };
    let Some(Binary::Map(shortcuts)) = entry(&root, "shortcuts") else {
        return Vec::new();
    };
    shortcuts
        .iter()
        .filter_map(|(_, shortcut)| {
            let Binary::Map(fields) = shortcut else { return None };
            let Some(&Binary::Number(id)) = entry(fields, "appid") else { return None };
            let exe = Some(PathBuf::from(text(fields, "exe").trim().trim_matches('"')))
                .filter(|exe| exe.is_absolute());
            let folder = exe.as_deref().and_then(Path::parent).map(Path::to_path_buf);
            Some(Game { id, title: text(fields, "appname").to_string(), folder, exe })
        })
        .collect()
}

/// A value in Valve's binary KeyValues: a map, text, or a 32-bit number.
#[derive(Debug, PartialEq)]
enum Binary {
    Map(Vec<(String, Binary)>),
    Text(String),
    Number(u32),
}

/// The entry named `key` in a map, its letter case ignored: Steam has written both `appname` and
/// `AppName`, `exe` and `Exe`.
fn entry<'a>(map: &'a [(String, Binary)], key: &str) -> Option<&'a Binary> {
    map.iter().find(|(name, _)| name.eq_ignore_ascii_case(key)).map(|(_, value)| value)
}

/// The text entry named `key` in a map ([`entry`]), empty where there's none.
fn text<'a>(map: &'a [(String, Binary)], key: &str) -> &'a str {
    match entry(map, key) {
        Some(Binary::Text(text)) => text,
        _ => "",
    }
}

/// The map starting at `at` in binary KeyValues, through its end mark; `at` moves past it. Each
/// entry is a type byte, a NUL-ended name, then the value: a map (0), NUL-ended text (1), or a
/// little-endian 32-bit number (2); [`BINARY_MAP_END`] ends the map. `None` for another type, or
/// data that ends first.
fn binary_map(data: &[u8], at: &mut usize) -> Option<Vec<(String, Binary)>> {
    let mut map = Vec::new();
    loop {
        let kind = *data.get(*at)?;
        *at += 1;
        if kind == BINARY_MAP_END {
            return Some(map);
        }
        let name = binary_text(data, at)?;
        let value = match kind {
            0 => Binary::Map(binary_map(data, at)?),
            1 => Binary::Text(binary_text(data, at)?),
            2 => {
                let bytes: [u8; 4] = data.get(*at..*at + 4)?.try_into().ok()?;
                *at += 4;
                Binary::Number(u32::from_le_bytes(bytes))
            }
            _ => return None,
        };
        map.push((name, value));
    }
}

/// The NUL-ended text starting at `at`, as UTF-8 (lossily); `at` moves past its NUL.
fn binary_text(data: &[u8], at: &mut usize) -> Option<String> {
    let rest = data.get(*at..)?;
    let end = rest.iter().position(|&byte| byte == 0)?;
    *at += end + 1;
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

/// A Steam library's own folder, in it.
const STEAMAPPS_DIR: &str = "steamapps";
/// Where a library keeps its games, each in the folder its manifest names: in `steamapps`.
const COMMON_DIR: &str = "common";
/// Where a library keeps its games' Proton prefixes, each named for its game's id.
const COMPATDATA_DIR: &str = "steamapps/compatdata";
/// An app manifest's name before its app's id, in `steamapps`.
const MANIFEST_PREFIX: &str = "appmanifest_";
/// An app manifest's name after its app's id.
const MANIFEST_SUFFIX: &str = ".acf";
/// Where Steam keeps a folder per account, named for the account's id: in Steam's own folder.
const USERDATA_DIR: &str = "userdata";
/// The non-Steam games an account added to Steam: in its folder.
const SHORTCUTS_FILE: &str = "config/shortcuts.vdf";
/// The byte that ends a map in binary KeyValues.
const BINARY_MAP_END: u8 = 8;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::proton::tests::scratch;

    /// A `shortcuts.vdf` the way Steam writes one: binary KeyValues, a `shortcuts` map holding a map
    /// per game (`0`, `1`, …) — its id (left out for `None`), its title and .exe under `keys` (Steam
    /// has written `AppName` and `Exe`, and both in lower case), a number and its tags.
    pub(crate) fn shortcuts_vdf(games: &[(Option<u32>, &str, &str)], keys: [&str; 2]) -> Vec<u8> {
        let mut vdf = b"\x00shortcuts\x00".to_vec();
        for (index, (id, title, exe)) in games.iter().enumerate() {
            vdf.extend(format!("\x00{index}\x00").bytes());
            if let Some(id) = id {
                vdf.extend(b"\x02appid\x00");
                vdf.extend(id.to_le_bytes());
            }
            vdf.extend(
                format!("\x01{}\x00{title}\x00\x01{}\x00{exe}\x00", keys[0], keys[1]).bytes(),
            );
            vdf.extend(b"\x02LastPlayTime\x00\x10\x00\x00\x00");
            vdf.extend(b"\x00tags\x00\x010\x00favorite\x00\x08");
            vdf.push(BINARY_MAP_END);
        }
        vdf.extend([BINARY_MAP_END, BINARY_MAP_END]);
        vdf
    }

    /// A library path is the `"path"` value as Valve writes it, backslashes undone.
    #[test]
    fn library_paths_are_read_from_the_vdf() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/u/.local/share/Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games\\\\x\"\n\t}\n}\n";
        assert_eq!(
            library_paths(vdf),
            [PathBuf::from("/home/u/.local/share/Steam"), PathBuf::from("/mnt/games\\x")]
        );
        assert!(library_paths("").is_empty());
    }

    /// A manifest's pairs are read at any depth, their escapes undone; a section's name and a
    /// string left open are none. The app is its id, title and install folder; no id, no app.
    #[test]
    fn an_app_manifest_reads_into_its_id_title_and_folder() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"620\"\n\t\"name\"\t\t\"Portal \\\"2\\\" \\\\ more\"\n\t\"installdir\"\t\t\"Portal 2\"\n\t\"UserConfig\"\n\t{\n\t\t\"language\"\t\t\"english\"\n\t}\n\t\"broken\"\t\t\"no end\n}\n";
        let pair = |key: &str, value: &str| (key.to_string(), value.to_string());
        assert_eq!(
            vdf_pairs(acf),
            [
                pair("appid", "620"),
                pair("name", "Portal \"2\" \\ more"),
                pair("installdir", "Portal 2"),
                pair("language", "english")
            ]
        );
        let steamapps = Path::new("/library/steamapps");
        let portal = Game {
            id: 620,
            title: "Portal \"2\" \\ more".into(),
            folder: Some(PathBuf::from("/library/steamapps/common/Portal 2")),
            exe: None,
        };
        assert_eq!(manifest_game(acf, steamapps), Some(portal));
        assert_eq!(manifest_game("\"AppState\"\n{\n\t\"name\"\t\t\"no id\"\n}\n", steamapps), None);
    }

    /// The non-Steam games come out of the binary file with the id Steam gave each, under either
    /// spelling of its keys, and their .exe and its folder, quoted or not, named in any script; one
    /// with no id is passed over, one that is no file has no folder, and a file cut short, or
    /// holding a type this doesn't read, gives none.
    #[test]
    fn non_steam_games_are_read_from_the_accounts_shortcuts_vdf() {
        let games = [
            (
                Some(3_640_112_877),
                "itchio-Cirno-ice-shave",
                "\"/games/ice/チルノの最強かき氷大作戦！/Game.exe\"",
            ),
            (Some(3_860_769_506), "Tool.exe", "/tools/tool.exe"),
            (None, "No Id.exe", "/x/no-id.exe"),
            (Some(2_200_000_000), "Flatpak", "flatpak"),
        ];
        let game = |id, title: &str, exe: Option<&str>| Game {
            id,
            title: title.into(),
            folder: exe.and_then(|exe| Path::new(exe).parent()).map(Path::to_path_buf),
            exe: exe.map(PathBuf::from),
        };
        let expected = [
            game(
                3_640_112_877,
                "itchio-Cirno-ice-shave",
                Some("/games/ice/チルノの最強かき氷大作戦！/Game.exe"),
            ),
            game(3_860_769_506, "Tool.exe", Some("/tools/tool.exe")),
            game(2_200_000_000, "Flatpak", None),
        ];
        let whole = shortcuts_vdf(&games, ["AppName", "Exe"]);
        assert_eq!(shortcuts_in(&whole), expected);
        assert_eq!(
            shortcuts_in(&shortcuts_vdf(&games, ["appname", "exe"])),
            expected,
            "the older spelling"
        );
        assert!(shortcuts_in(&whole[..whole.len() - 3]).is_empty(), "cut short");
        assert!(shortcuts_in(b"").is_empty());
        assert!(
            shortcuts_in(b"\x00shortcuts\x00\x09odd\x00\x08\x08").is_empty(),
            "a type it doesn't read"
        );
    }

    /// A Steam home: a library besides Steam's own folder (which `libraryfolders.vdf` lists too), a
    /// game installed in each, one with its prefix and its Steam Cloud folder, and the non-Steam
    /// games an account added, one in a folder named in Japanese, with its prefix in Steam's own
    /// library. Each library is gathered once; each game's installs, title, prefixes and Steam
    /// Cloud folders are those that are there.
    #[test]
    fn steam_is_gathered_with_its_libraries_games_prefixes_and_cloud_folders() {
        let home = scratch("steam");
        let steam = home.join(".local/share/Steam");
        let disk = home.join("disk");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        let listed = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            steam.display(),
            disk.display()
        );
        std::fs::write(steam.join("steamapps/libraryfolders.vdf"), listed).unwrap();
        let install = |library: &Path, id: u32, title: &str, folder: &str| {
            std::fs::create_dir_all(library.join("steamapps/common").join(folder)).unwrap();
            let acf = format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{title}\"\n\t\"installdir\"\t\t\"{folder}\"\n}}\n"
            );
            std::fs::write(library.join(format!("steamapps/appmanifest_{id}.acf")), acf).unwrap();
        };
        install(&disk, 620, "Portal 2", "Portal 2");
        install(&steam, 400, "Portal", "Portal");
        let account = steam.join("userdata/76197245");
        std::fs::create_dir_all(account.join("config")).unwrap();
        std::fs::create_dir_all(account.join("620/remote")).unwrap();
        let ice = home.join("games/ice/チルノの最強かき氷大作戦！");
        std::fs::create_dir_all(&ice).unwrap();
        let exe = format!("\"{}\"", ice.join("Game.exe").display());
        let added = [
            (Some(3_640_112_877), "itchio-Cirno-ice-shave", exe.as_str()),
            (Some(3_000_000_000), "Portal 2.exe", "/gone/Portal 2.exe"),
        ];
        std::fs::write(
            account.join("config/shortcuts.vdf"),
            shortcuts_vdf(&added, ["AppName", "Exe"]),
        )
        .unwrap();
        for prefix in
            [disk.join("steamapps/compatdata/620"), steam.join("steamapps/compatdata/3640112877")]
        {
            std::fs::create_dir_all(prefix).unwrap();
        }

        let known = Known::gather(&home);
        assert_eq!(
            known.libraries,
            [steam.clone(), disk.clone()],
            "Steam's own library once, though listed again"
        );
        assert_eq!(known.accounts, std::slice::from_ref(&account));
        let ids: Vec<u32> = known.games.iter().map(|game| game.id).collect();
        assert_eq!(
            ids,
            [400, 620, 3_640_112_877, 3_000_000_000],
            "the libraries' apps, then the accounts' non-Steam games"
        );
        assert_eq!(known.installs_of(620), [disk.join("steamapps/common/Portal 2")]);
        assert_eq!(known.installs_of(3_640_112_877), [ice]);
        assert!(known.installs_of(3_000_000_000).is_empty(), "its folder is gone");
        assert_eq!(known.title_of(3_640_112_877), "itchio-Cirno-ice-shave");
        assert_eq!(known.prefixes_of(620), [disk.join("steamapps/compatdata/620")]);
        assert_eq!(
            known.prefixes_of(3_640_112_877),
            [steam.join("steamapps/compatdata/3640112877")]
        );
        assert_eq!(known.cloud_folders_of(620), [account.join("620")]);
        assert!(known.prefixes_of(999).is_empty() && known.cloud_folders_of(999).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}
