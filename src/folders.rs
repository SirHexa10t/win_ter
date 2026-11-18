//! The folders too broad for a game to write to: one that is, or holds, the user's own folders
//! ([`users_folders`]: the home, the folders their files and their programs' data live in, the
//! folder every game's own is made in), or where a filesystem is mounted ([`mount_points`]) — the
//! game could write to all of it. [`too_broad`] says which and why.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::prefix::PREFIXES_DIR;

/// The user's own folders, which no folder the game may write to may be or hold, each with how a
/// message names it: the home; the folder every game's own is made in; where programs keep their
/// settings, data, state and caches (`XDG_CONFIG_HOME` and the rest, read through `env`, else
/// their usual places); and the folders the user's files live in ([`user_dirs`]). Each with its
/// links resolved; `home` is too.
pub(crate) fn users_folders(
    home: &Path,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<(String, PathBuf)> {
    let base = |key: &str, usual: &str| {
        env(key)
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(|| home.join(usual))
    };
    let config = base("XDG_CONFIG_HOME", ".config");
    let user_dirs = user_dirs(
        &std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default(),
        home,
    );
    let programs = [
        config,
        base("XDG_DATA_HOME", ".local/share"),
        base("XDG_STATE_HOME", ".local/state"),
        base("XDG_CACHE_HOME", ".cache"),
    ];
    let prefixes = home.join(PREFIXES_DIR);
    let named = |dir: PathBuf| (format!("{}, one of your own folders", shown(&dir, home)), dir);
    [
        ("your home folder".to_string(), home.to_path_buf()),
        (format!("{}, where every game's own folder is made", shown(&prefixes, home)), prefixes),
    ]
    .into_iter()
    .chain(programs.into_iter().chain(user_dirs).map(named))
    .map(|(label, dir)| (label, crate::canonical(&dir)))
    .collect()
}

/// The folders a `user-dirs.dirs` names, a line each (`XDG_DOWNLOAD_DIR="$HOME/Downloads"`),
/// `$HOME` read as `home` — the home itself left out, as one switched off is set to it; where it
/// names none (no file, or nothing in it), their usual names in the home ([`USUAL_USER_DIRS`]).
fn user_dirs(text: &str, home: &Path) -> Vec<PathBuf> {
    let named: Vec<PathBuf> = text
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            if !(key.starts_with("XDG_") && key.ends_with("_DIR")) {
                return None;
            }
            let value = value.trim().trim_matches('"');
            let dir = match value.strip_prefix("$HOME") {
                Some(rest) => home.join(rest.trim_start_matches('/')),
                None => PathBuf::from(value),
            };
            (dir.is_absolute() && dir.as_path() != home).then_some(dir)
        })
        .collect();
    match named.is_empty() {
        true => USUAL_USER_DIRS.iter().map(|name| home.join(name)).collect(),
        false => named,
    }
}

/// What a folder the game may write to mayn't be, said as "it is …" or "it holds …", if so: one of
/// the user's own folders (`own`, [`users_folders`]), or where a filesystem is mounted (`mounts`),
/// or a folder holding one.
pub(crate) fn too_broad(
    folder: &Path,
    own: &[(String, PathBuf)],
    mounts: &[PathBuf],
) -> Option<String> {
    let folder = crate::canonical(folder);
    let mounted = mounts
        .iter()
        .map(|dir| (format!("{}, where a filesystem is mounted", dir.display()), dir.clone()));
    own.iter().cloned().chain(mounted).find(|(_, dir)| dir.starts_with(&folder)).map(
        |(label, dir)| {
            let verb = if dir == folder { "is" } else { "holds" };
            format!("it {verb} {label}")
        },
    )
}

/// Where filesystems are mounted, as this process sees them ([`mount_points_in`]).
pub(crate) fn mount_points() -> Vec<PathBuf> {
    mount_points_in(&std::fs::read("/proc/self/mountinfo").unwrap_or_default())
}

/// The mount points a `mountinfo` names: each line's fifth field, the kernel's octal escapes
/// undone (`\040` a space, `\011` a tab, `\012` a line break, `\134` a backslash).
fn mount_points_in(info: &[u8]) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let unescaped = |field: &[u8]| {
        let mut path = Vec::with_capacity(field.len());
        let mut at = 0;
        while at < field.len() {
            let is_octal = |digits: &&[u8]| {
                field[at] == b'\\' && digits.iter().all(|digit| (b'0'..=b'7').contains(digit))
            };
            match field.get(at + 1..at + 4).filter(is_octal) {
                Some(digits) => {
                    let byte = digits
                        .iter()
                        .fold(0u8, |byte, digit| byte.wrapping_mul(8).wrapping_add(digit - b'0'));
                    path.push(byte);
                    at += 4;
                }
                None => {
                    path.push(field[at]);
                    at += 1;
                }
            }
        }
        PathBuf::from(OsString::from_vec(path))
    };
    info.split(|&byte| byte == b'\n')
        .filter_map(|line| line.split(|&byte| byte == b' ').nth(4))
        .map(unescaped)
        .collect()
}

/// `path` as a message shows it: from `~` where it's in the home.
fn shown(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The usual names of the folders the user's files live in, where `user-dirs.dirs` names none.
const USUAL_USER_DIRS: &[&str] =
    &["Desktop", "Documents", "Downloads", "Music", "Pictures", "Public", "Templates", "Videos"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proton::tests::scratch;

    /// A folder the game may write to mayn't be, or hold, the user's own folders — the home; the
    /// folders `user-dirs.dirs` names, wherever they are, else their usual ones; the programs'
    /// settings, data, state and caches, where the environment says or usually; the folder every
    /// game's own is made in — or where a filesystem is mounted. A folder inside one of them is a
    /// game's own, and fine.
    #[test]
    fn a_folder_the_game_writes_to_mustnt_be_or_hold_the_users_own() {
        let home = crate::canonical(&scratch("broad"));
        std::fs::create_dir_all(home.join(".config")).unwrap();
        let user_dirs = "# xdg-user-dirs-update\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\nXDG_MUSIC_DIR=\"/mnt/music\"\nXDG_DESKTOP_DIR=\"$HOME/\"\n";
        std::fs::write(home.join(".config/user-dirs.dirs"), user_dirs).unwrap();
        let env = |key: &str| match key {
            "XDG_CACHE_HOME" => Some(OsString::from("/var/cache/u")),
            "XDG_DATA_HOME" => Some(OsString::from("relative, so not taken")),
            _ => None,
        };
        let own = users_folders(&home, &env);
        let mounts = [PathBuf::from("/"), PathBuf::from("/mnt/games")];
        let why = |folder: &Path| too_broad(folder, &own, &mounts);
        let said = |text: &str| Some(text.to_string());
        assert_eq!(why(&home), said("it is your home folder"));
        assert_eq!(why(home.parent().unwrap()), said("it holds your home folder"));
        assert_eq!(
            why(&home.join("Téléchargements")),
            said("it is ~/Téléchargements, one of your own folders")
        );
        assert_eq!(why(Path::new("/mnt/music")), said("it is /mnt/music, one of your own folders"));
        assert_eq!(why(&home.join(".config")), said("it is ~/.config, one of your own folders"));
        assert_eq!(
            why(&home.join(".local")),
            said("it holds ~/.local/share, one of your own folders")
        );
        assert_eq!(
            why(Path::new("/var/cache/u")),
            said("it is /var/cache/u, one of your own folders")
        );
        assert_eq!(
            why(&home.join("Games")),
            said(
                "it holds ~/Games/manual_run_proton_prefixes, where every game's own folder is made"
            )
        );
        assert_eq!(
            why(Path::new("/mnt/games")),
            said("it is /mnt/games, where a filesystem is mounted")
        );
        assert_eq!(why(Path::new("/mnt")), said("it holds /mnt/music, one of your own folders"));
        for fine in [
            home.join("Downloads"),
            home.join("Téléchargements/Some Game"),
            home.join(".config/itch/apps/ice"),
            home.join("Games/Some Game"),
            home.join("Games/manual_run_proton_prefixes/Some Game-1"),
            PathBuf::from("/mnt/games/steamapps/common/G"),
        ] {
            assert_eq!(why(&fine), None, "{}", fine.display());
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// With no `user-dirs.dirs`, or none named in it, the usual folders are the user's.
    #[test]
    fn the_usual_folders_are_the_users_where_none_are_named() {
        let home = Path::new("/home/u");
        let usual: Vec<PathBuf> = USUAL_USER_DIRS.iter().map(|name| home.join(name)).collect();
        assert_eq!(user_dirs("", home), usual);
        assert_eq!(user_dirs("# nothing but comments\nOTHER=1\n", home), usual);
        assert_eq!(
            user_dirs("XDG_VIDEOS_DIR=\"$HOME/Clips\"\n", home),
            [PathBuf::from("/home/u/Clips")]
        );
    }

    /// Mount points are each line's fifth field, a space and the like unescaped.
    #[test]
    fn mount_points_are_read_from_mountinfo() {
        let info = b"21 1 0:19 / / rw - btrfs /dev/x rw\n36 21 0:30 / /run/media/u/My\\040Disk rw - ext4 /dev/y rw\n\n";
        assert_eq!(
            mount_points_in(info),
            [PathBuf::from("/"), PathBuf::from("/run/media/u/My Disk")]
        );
        assert!(mount_points_in(b"").is_empty());
        assert!(
            mount_points().iter().any(|point| point == Path::new("/")),
            "this process sees its root mounted"
        );
    }
}
