//! `serve::command`: what the command refuses before it touches a database (piece 11 `serve` S4c, mira). DB-free. RED first.
//! This binary calls `command` and nothing else that takes the process's one `ProblemCodeMinter`: only `sc3` gets as far as assembling.
use std::fs;
use std::path::{Path, PathBuf};

use cortex_server::serve::command;

fn write_config(name: &str, text: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("sc-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    let f = d.join("cortex.toml");
    fs::write(&f, text).expect("config");
    f
}

async fn refusal(path: &Path) -> String {
    command(path, None)
        .await
        .expect_err("the command is refused")
        .to_string()
}

fn one_line(text: &str) -> bool {
    !text.is_empty() && !text.contains('\n')
}

#[tokio::test]
async fn sc1_a_config_that_cannot_be_read_or_parsed_is_refused_in_one_line_without_the_path_or_the_content()
 {
    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("sc-no-such-dir/canary-config-dir/cortex.toml");
    let e = refusal(&missing).await;
    assert!(e.starts_with("serve refused: configuration: "), "{e}");
    assert!(one_line(&e) && !e.contains("canary-config-dir"), "{e}");

    let bad = write_config("bad", "[modules]\nenabled = \"canary-toml-value\" = =\n");
    let e = refusal(&bad).await;
    assert!(e.starts_with("serve refused: configuration: "), "{e}");
    assert!(one_line(&e) && !e.contains("canary-toml-value"), "{e}");

    // An unknown key is named in the error, and a quoted TOML key may hold line breaks: the refusal is still ONE line.
    let multiline = write_config(
        "multiline",
        "\"line\\none\\rtwo\" = 1\n[modules]\nenabled = [\"identity\"]\n",
    );
    let e = refusal(&multiline).await;
    assert!(e.starts_with("serve refused: configuration: "), "{e}");
    assert!(one_line(&e) && !e.contains('\r'), "{e:?}");
    assert!(
        e.contains("line one two"),
        "the breaks became spaces: {e:?}"
    );

    let unknown = write_config(
        "unknown",
        "[modules]\nenabled = [\"identity\"]\n[nonsense]\nx = 1\n",
    );
    let e = refusal(&unknown).await;
    assert!(e.starts_with("serve refused: configuration: "), "{e}");
    assert!(one_line(&e), "{e}");
}

#[tokio::test]
async fn sc2_a_module_enabled_twice_is_refused_naming_it() {
    let p = write_config(
        "twice",
        "[modules]\nenabled = [\"identity\", \"records\", \"identity\"]\n",
    );
    assert_eq!(
        refusal(&p).await,
        "serve refused: module identity is enabled twice"
    );
}

#[tokio::test]
async fn sc3_an_enabled_module_that_is_not_compiled_in_is_refused_with_the_startup_line() {
    let p = write_config("unknown-module", "[modules]\nenabled = [\"nonesuch\"]\n");
    let e = refusal(&p).await;
    assert!(e.starts_with("serve refused: "), "{e}");
    assert!(e.contains("nonesuch") && one_line(&e), "{e}");
}
