//! Sibling resource-container discovery.
//!
//! Titles that ship a companion resource container keep it next to the
//! `.app` package, and every frontend serves it the same way: an exact
//! `<content-stem>.pak`/`<content-stem>.res` sibling wins, and otherwise
//! every sibling whose name mentions `.pak` or `res` is served. A title
//! may carry more than one container (each with its own pack), so every
//! fuzzy match is served rather than only an unambiguous one.
//!
//! Only the name matching lives here — the caller walks the directory and
//! reads the files it names, keeping host filesystem details out of the
//! core.

/// Returns the sibling names to serve as resource containers, sorted.
///
/// `content_stem` is the content file's stem (without extension); the
/// exact `<stem>.pak`/`<stem>.res` candidates are checked first and
/// short-circuit the fuzzy scan. `content_name` is the content file's own
/// name and is never returned: a content file whose own name mentions
/// `res` must not be served as its own container.
pub fn sibling_res_candidates(
    content_stem: Option<&str>,
    content_name: &str,
    dir_names: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let names: Vec<String> = dir_names.into_iter().collect();
    if let Some(stem) = content_stem {
        for extension in [".pak", ".res"] {
            let candidate = format!("{stem}{extension}");
            if candidate != content_name && names.iter().any(|name| name == &candidate) {
                return vec![candidate];
            }
        }
    }
    let mut matches: Vec<String> = names
        .into_iter()
        .filter(|name| {
            name != content_name
                && (name.to_ascii_lowercase().contains(".pak")
                    || name.to_ascii_lowercase().contains("res"))
        })
        .collect();
    matches.sort();
    matches
}

#[cfg(test)]
mod tests {
    use super::sibling_res_candidates;

    #[test]
    fn exact_stem_sibling_wins_over_the_fuzzy_scan() {
        let names = [
            "gameRes_QVGA_SNOWFISH.app",
            "game.pak",
            "resource notes.txt",
        ]
        .map(String::from);
        let mut candidates = sibling_res_candidates(Some("game"), "game.app", names.clone());
        candidates.sort();
        assert_eq!(candidates, vec!["game.pak".to_string()]);
    }

    #[test]
    fn prefers_pak_over_res_for_the_same_stem() {
        let names = ["game.res", "game.pak"].map(String::from);
        assert_eq!(
            sibling_res_candidates(Some("game"), "game.app", names),
            vec!["game.pak".to_string()]
        );
    }

    #[test]
    fn fuzzy_scan_serves_every_container_sorted() {
        let names = [
            "Mengde_iRes2_QVGA_SNOWFISH.app",
            "Mengde_iRes1_QVGA_SNOWFISH.app",
            "游戏简介.txt",
            "Mengde_i.gif",
        ]
        .map(String::from);
        assert_eq!(
            sibling_res_candidates(
                Some("Mengde_i_QVGA_SNOWFISHV2"),
                "Mengde_i_QVGA_SNOWFISHV2.app",
                names
            ),
            vec![
                "Mengde_iRes1_QVGA_SNOWFISH.app".to_string(),
                "Mengde_iRes2_QVGA_SNOWFISH.app".to_string(),
            ]
        );
    }

    #[test]
    fn matching_is_case_insensitive_and_the_content_is_excluded() {
        let names = ["MyResources.bin", "content.pak", "game.APP"].map(String::from);
        assert_eq!(
            sibling_res_candidates(Some("game"), "game.APP", names),
            vec!["MyResources.bin".to_string(), "content.pak".to_string()]
        );
    }

    #[test]
    fn no_stem_still_scans_fuzzily() {
        let names = ["pack.pak", "unrelated.txt"].map(String::from);
        assert_eq!(
            sibling_res_candidates(None, "", names),
            vec!["pack.pak".to_string()]
        );
    }

    #[test]
    fn a_res_named_content_is_never_its_own_container() {
        let names = ["foo.res", "bar.pak"].map(String::from);
        assert_eq!(
            sibling_res_candidates(Some("foo"), "foo.res", names),
            vec!["bar.pak".to_string()]
        );
    }
}
