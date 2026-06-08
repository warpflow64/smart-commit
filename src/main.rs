use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::sleep,
    time::{Duration, Instant},
};

// ---- CLI ---------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "sc",
    about = "Ollamaでコミットメッセージを自動生成してpushするツール",
    version
)]
struct Cli {
    /// 使用するOllamaモデル
    #[arg(short, long, default_value = "gemma4:e2b")]
    model: String,

    /// pushをスキップする
    #[arg(long)]
    no_push: bool,

    /// コミットメッセージの確認プロンプトを表示する
    #[arg(short, long)]
    interactive: bool,

    /// コミットメッセージの言語 (例: ja, en)
    #[arg(short, long, default_value = "ja")]
    lang: String,

    /// diffの最大文字数
    #[arg(long, default_value_t = 4000)]
    diff_limit: usize,
}

// ---- Ollama API --------------------------------------------------------

const OLLAMA_BASE: &str = "http://localhost:11434";
const STARTUP_TIMEOUT_SECS: u64 = 30;

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

fn is_ollama_running() -> bool {
    ureq::get(&format!("{}/api/tags", OLLAMA_BASE))
        .timeout(Duration::from_secs(2))
        .call()
        .is_ok()
}

fn start_ollama() -> Result<Child> {
    eprintln!("[sc] Ollama起動中...");
    let child = Command::new("ollama")
        .arg("serve")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("ollama の起動に失敗しました。インストールされていますか？")?;

    let deadline = Instant::now() + Duration::from_secs(STARTUP_TIMEOUT_SECS);
    while Instant::now() < deadline {
        if is_ollama_running() {
            eprintln!("[sc] Ollama起動完了");
            return Ok(child);
        }
        sleep(Duration::from_millis(200));
    }
    bail!("Ollama起動がタイムアウトしました ({STARTUP_TIMEOUT_SECS}秒)");
}

fn generate_message(model: &str, diff: &str, lang: &str) -> Result<String> {
    let lang_note = match lang {
        "ja" => "日本語で",
        "en" => "in English",
        other => other,
    };

    let prompt = format!(
        "You are a commit message generator. Output ONLY the commit message.\n\
         Rules:\n\
         - Use Conventional Commits format: type(scope): description\n\
         - Types: feat, fix, refactor, docs, test, chore, style, perf, ci, build\n\
         - Language: {lang_note}\n\
         - Single line, no quotes, no markdown, no explanation\n\n\
         Git diff:\n{diff}"
    );

    let req = GenerateRequest {
        model,
        prompt,
        stream: false,
    };

    let resp: GenerateResponse = ureq::post(&format!("{}/api/generate", OLLAMA_BASE))
        .timeout(Duration::from_secs(60))
        .send_json(&req)
        .context("Ollama APIへのリクエストに失敗しました")?
        .into_json()
        .context("Ollamaのレスポンスのパースに失敗しました")?;

    let msg = resp.response.trim().to_string();
    if msg.is_empty() {
        bail!("Ollamaが空のレスポンスを返しました");
    }
    Ok(msg)
}

// ---- Git ---------------------------------------------------------------

fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn get_staged_diff(limit: usize) -> Result<String> {
    let out = Command::new("git")
        .args(["diff", "--cached"])
        .output()
        .context("git diff の実行に失敗しました")?;

    if !out.status.success() {
        bail!("git diff が失敗しました");
    }

    let full = String::from_utf8_lossy(&out.stdout);
    Ok(full.chars().take(limit).collect())
}

fn git_commit(message: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["commit", "-m", message])
        .status()
        .context("git commit の実行に失敗しました")?;

    if !status.success() {
        bail!("git commit が失敗しました");
    }
    Ok(())
}

fn git_push() -> Result<()> {
    let branch_out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("ブランチ名の取得に失敗しました")?;

    let branch = String::from_utf8_lossy(&branch_out.stdout)
        .trim()
        .to_string();

    eprintln!("[sc] push中: origin/{branch}");

    let status = Command::new("git")
        .args(["push", "--set-upstream", "origin", &branch])
        .status()
        .context("git push の実行に失敗しました")?;

    if !status.success() {
        bail!("git push が失敗しました");
    }
    Ok(())
}

// ---- Main --------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    // gitリポジトリ内かチェック
    if !is_git_repo() {
        bail!("gitリポジトリではありません。gitリポジトリ内で実行してください。");
    }

    // staged diff を取得
    let diff = get_staged_diff(cli.diff_limit)?;
    if diff.trim().is_empty() {
        bail!("staged changes がありません。先に git add してください。");
    }

    // Ollama が起動していなければ自動起動
    let mut ollama_child: Option<Child> = None;
    if !is_ollama_running() {
        ollama_child = Some(start_ollama()?);
    }

    // クリーンアップを保証するためにクロージャで囲む
    let result = run(&cli, &diff);

    if let Some(mut child) = ollama_child {
        eprintln!("[sc] Ollama終了中...");
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM); }
        let _ = child.wait();
    }

    result
}

fn run(cli: &Cli, diff: &str) -> Result<()> {
    eprintln!("[sc] コミットメッセージ生成中 (model: {})...", cli.model);

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let spinner = std::thread::spawn(move || {
        let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let mut i = 0;
        while r.load(Ordering::Relaxed) {
            eprint!("\r  {} 生成中...", frames[i % frames.len()]);
            io::stderr().flush().ok();
            sleep(Duration::from_millis(80));
            i += 1;
        }
        eprint!("\r");
        io::stderr().flush().ok();
    });

    let mut msg = generate_message(&cli.model, diff, &cli.lang)?;

    running.store(false, Ordering::Relaxed);
    spinner.join().ok();
    eprintln!();

    eprintln!("\n    {msg}\n");

    // --interactive: 確認して編集・スキップできるようにする
    if cli.interactive {
        eprint!("このメッセージでコミットしますか? [Y/n/edit]: ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        match input.trim() {
            "n" | "N" => bail!("キャンセルしました"),
            "e" | "edit" => {
                eprint!("新しいメッセージを入力: ");
                let mut new_msg = String::new();
                std::io::stdin().read_line(&mut new_msg)?;
                msg = new_msg.trim().to_string();
            }
            _ => {}
        }
    }

    git_commit(&msg)?;
    eprintln!("[sc] コミット完了");

    if !cli.no_push {
        git_push()?;
        eprintln!("[sc] push完了");
    }

    Ok(())
}
