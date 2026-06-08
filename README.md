# smart-commit

Ollamaでgitコミットメッセージを自動生成してpushするCLIツール。

## インストール

```bash
cargo install smart-commit
```

## 前提

- [Ollama](https://ollama.com) がインストールされていること
- モデルが事前にpull済みであること

```bash
ollama pull gemma4:e2b   # デフォルト
# または
ollama pull llama3.2
```

Ollamaが起動していなくても、実行時に自動で起動・終了します。

## 使い方

```bash
# 基本: add済みの変更をコミット + push
git add .
sc

# pushしない
sc --no-push

# 確認プロンプトあり
sc --interactive

# モデルを変える
sc --model llama3.2

# 英語メッセージ
sc --lang en
```

## オプション

| フラグ | 短縮 | デフォルト | 説明 |
|---|---|---|---|
| `--model` | `-m` | `gemma4:e2b` | Ollamaモデル名 |
| `--lang` | `-l` | `ja` | メッセージ言語 |
| `--no-push` | - | false | pushをスキップ |
| `--interactive` | `-i` | false | 確認プロンプト |
| `--diff-limit` | - | 4000 | diffの最大文字数 |

## 動作フロー

```
git add .  →  sc
              |
              Ollamaが起動していなければ自動起動
              |
              git diff --cached を取得
              |
              Ollama API でコミットメッセージ生成
              |
              git commit -m "生成されたメッセージ"
              |
              git push origin <現在のブランチ>
              |
              (自分で起動したOllamaを終了)
```

## ライセンス

MIT
