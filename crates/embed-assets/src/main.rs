#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::struct_excessive_bools
)]

mod embed;
mod scan;
mod srt;
mod ui;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

use crate::scan::{run, Options};

#[derive(Parser, Debug)]
#[command(
    name = "embed-assets",
    version,
    about = "把同名 .jpg/.png/.lrc 嵌入相邻音频文件 tag，并把消耗掉的源文件归集到扫描根目录下的 _used/"
)]
struct Args {
    /// 要扫描的目录；省略且 stdin 是终端时进入交互式输入
    dir: Option<PathBuf>,

    /// 不递归子目录
    #[arg(long)]
    no_recursive: bool,

    /// 即使已有 cover/lyrics 也覆盖
    #[arg(long)]
    force: bool,

    /// 仅打印将要做的事，不真正写盘 / 不移动
    #[arg(long)]
    dry_run: bool,

    /// 嵌入成功后不移动 jpg/lrc 到 _used/ 子目录
    #[arg(long)]
    no_move: bool,

    /// 多音频同 stem 时跳过交互直接合入所有
    #[arg(short = 'y', long)]
    yes: bool,
}

fn main() -> ExitCode {
    let interactive = run_main();
    let was_interactive = matches!(&interactive, Ok(true) | Err(_)) && io::stdin().is_terminal();
    let code = match interactive {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    };
    if was_interactive {
        ui::pause();
    }
    code
}

/// 返回 Ok(true) 表示走了交互路径（结束时需 pause），Ok(false) 表示纯 CLI 调用。
fn run_main() -> Result<bool> {
    let args = Args::parse();
    let (root_input, interactive) = match args.dir {
        Some(p) => (p, false),
        None if io::stdin().is_terminal() => (ui::prompt_dir()?, true),
        None => (PathBuf::from("."), false),
    };

    let root = root_input
        .canonicalize()
        .ok()
        .unwrap_or_else(|| root_input.clone());

    println!(
        "Scanning {} ({})...",
        root.display(),
        if args.no_recursive {
            "single dir"
        } else {
            "recursive"
        }
    );

    let opts = Options {
        recursive: !args.no_recursive,
        force: args.force,
        dry_run: args.dry_run,
        no_move: args.no_move,
        assume_yes: args.yes,
    };

    let stats =
        run(&root, &opts).with_context(|| format!("scan failed under {}", root.display()))?;

    println!(
        "\nDone. {} modified, {} unchanged, {} error, {} assets moved, {} deduped, {} groups skipped, {} files scanned.",
        stats.modified,
        stats.unchanged,
        stats.errored,
        stats.assets_moved,
        stats.assets_deduped,
        stats.groups_skipped,
        stats.scanned
    );
    Ok(interactive)
}
