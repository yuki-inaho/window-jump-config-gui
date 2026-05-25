# LLMオンボーディングサマリー — window-jump

> 本書は、新任LLMエージェントが `window-jump` プロジェクトへ参加する際の初期資料です。
> まず本書を上から読み、**「0. オンボーディング実施手順」の順序どおりに現状を自分の目で確認**してから作業に着手してください。
> 最終更新: 2026-05-25 / 対象: `main` ブランチ
> 最新状態は `git status -sb` と `git log -1 --oneline` で必ず確認してください。

---

## 0. オンボーディング実施手順（この順序で進めること）

新任エージェントは、いきなり実装に入らず、次の順序で「生成物・ドキュメント・状況」を確認してください。各ステップは前のステップの結果に依存します。

1. **本書（docs/ONBOARDING.md）を通読する。** 特に「2. クリティカルな要求・制約」を頭に入れる。これがレッドラインです。
2. **README.md を読む。** これがユーザー向けの唯一の一次ドキュメントです。前提・配置・GNOMEショートカット登録・設定ファイル仕様・マッチング仕様・トラブルシュートが書かれています。
3. **`Cargo.toml` と `justfile` を読む。** 依存クレートと、開発・運用で使う正規コマンド（`just doctor` / `build` / `check` / `verify-local` など）を把握します。
4. **ソースを次の順で読む（依存の浅い順）:**
   - `src/bin/window-jump.rs`（CLIエントリ・3行）
   - `src/bin/window-jump-config-gui.rs`（GUIエントリ・引数処理）
   - `src/ui.rs`（日本語フォント自動検出）
   - `src/lib.rs`（本体。CLI・GUI・マッチング・永続化のすべて。最重要）
   `src/lib.rs` では特に `resolve_slot` / `rule_matches` / `suggest_title_contains` / `save_store` / `write_atomic` / `StoreLock` を重点的に読むこと。
5. **環境の前提を自分で確認する（後述「付録」のコマンド）:**
   - `echo "$XDG_SESSION_TYPE"` が `x11` か
   - `command -v wmctrl xdotool` が両方解決するか
   - **ツールチェーン状況**（後述の重要注意。`rustc` / `cargo` が 1.92 以上か）
6. **品質ゲートを再現する。** `just check`（fmt-check → clippy → test）を流し、現状が緑であることを確認する。テストは `src/lib.rs` に 11 件、`src/ui.rs` に 3 件、計 14 件。
7. **隔離スモークで挙動を確認する。** 本番設定を汚さずに、`XDG_CONFIG_HOME` を一時ディレクトリに向けて CLI を起動する（後述「6. 試行タスク」）。
8. **「4. タスク境界」を確認し、着手予定の作業が任せられる範囲か判断する。** 範囲外・未確定があれば、推測で進めず責任者に確認する。

> ⚠️ **現状の最重要事項（着手前に必読）— Rust 1.92 以上を使うこと**
> `eframe`/`egui` 0.34.1 系は **rustc 1.92 以上**を要求します。作業前に `cargo --version` と `rustc --version` を確認してください。
> 2026-05-25 時点のこの環境では、`/usr/bin/cargo` / `/usr/bin/rustc` ともに **1.95.0** で、`just check` と `cargo build --release --bins` は成功します。
> 古い `cargo` / `rustc` を拾う環境では、`rustup update stable` 後に `rustup run stable` 経由で実行してください:
> ```bash
> cargo --version
> rustc --version
> cargo build --release --bins
>
> # cargo/rustc が 1.92 未満の場合
> rustup update stable
> rustup run stable cargo build --release --bins
> ```

---

## 1. プロジェクト概要と目的
- **プロジェクト名称・領域:** `window-jump` — Ubuntu GNOME（**X11セッション専用**）向けウィンドウジャンプ／フォーカス切替ツール（Rust製CLI＋eframe/egui製設定GUI）。
- **最終成果物:** 2つのバイナリ。
  - `window-jump`（CLI）— `activate N` 等でスロットに紐づくウィンドウを前面化。
  - `window-jump-config-gui`（GUI）— スロット 1..9 に「再発見ルール」を登録・編集。
  GNOME Custom Shortcuts（例: `Ctrl+Alt+1` → `window-jump activate 1`）から呼び出して使う想定。
- **ビジネス背景・価値:** ウィンドウを **window id ではなく `WM_CLASS` + `title_contains`（部分一致）という再現性のあるルール**で記憶し、再起動やウィンドウ再生成をまたいで「いつものウィンドウ」へ一発で飛べるようにすること。誤って別ウィンドウを前面化しないこと（安全側に倒す）を重視。
- **現時点の進捗サマリ:**
  - 初期実装は `b95bd18 first commit`。最新のコミット・未コミット差分は `git status -sb` と `git log -1 --oneline` で確認すること。
  - CLI／GUI／マッチング／永続化（config・state分離、atomic write、ロック、旧形式マイグレーション）は実装済み。単体テスト 14 件あり。
  - **ビルド:** 2026-05-25 に Rust 1.95.0 でリリースビルド成功済み（`window-jump 0.1.0` / `window-jump-config-gui 0.1.0`）。ビルド前に `cargo --version` / `rustc --version` で 1.92 以上を確認すること（上の⚠️参照）。
  - 別ファイルの要求定義書・要件定義書・WBS・課題票は**存在しない**（一次資料は README・justfile・ソース＋テスト）。

## 2. クリティカルな要求・制約
> 「壊してはいけない」品質・仕様ラインです。変更時はここに抵触しないか必ず確認すること。

- **X11専用。** Wayland はサポート外。`wmctrl` / `xdotool` への外部コマンド呼び出しに依存する設計を前提にする（`list_windows` は `wmctrl -lpxG`、選択・前面化は `xdotool`）。
- **マッチングキーは window id ではなく `WM_CLASS` + `title_contains`。** 再現性を担保するこの方針を崩さない。`title_contains` は **大文字小文字を無視する部分一致**（完全一致ではない）。
- **曖昧時は安全側で失敗する。** `resolve_slot` は、複数候補が残り `last_window_id` / `desktop_hint` でも一意に絞れない場合、誤操作防止のため**エラーで失敗**する。この「黙って当てずっぽうで前面化しない」挙動を必ず維持する。
- **設定の壊さない保存。** 保存は一時ファイル＋`rename` の **atomic write**（`write_atomic`）で行い、`.window-jump.lock`（30秒で stale 判定）で排他する。この不変条件を壊さない。
- **`config.json` と `state.json` の分離・スキーマ互換。** `config.json` はルール、`state.json` は補助情報（`desktop_hint` / `last_window_id` / `last_seen_title`）。旧形式（config にstateが混在）からの **legacy migration**（`legacy_state_from_config_json`）を壊さない。`XDG_CONFIG_HOME` を尊重する。
- **読み込み失敗時は read-only 起動。** GUIは設定読込に失敗しても落とさず read-only で起動し、保存を無効化する。この防御挙動を維持する。
- **日本語UI。** GUI文言・ステータスは日本語。`src/ui.rs` のフォント自動検出（CJKフォント優先選択）を壊さない。
- **品質ゲートは常に緑。** `cargo fmt --check` / `cargo clippy --all-targets --all-features -- -D warnings`（警告ゼロ必須）/ `cargo test` の3点（= `just check`）を通すこと。**clippy の警告は許容しない。**
- **GUIの終了キー仕様。** `q` / `Esc` で終了。ただし**テキスト入力中は通常入力として扱う**（`ctx.text_edit_focused()` ガード）。この区別を壊さない。

## 3. 参照すべき合意済み資料
> このリポジトリには**フォーマルな要求/要件/WBS/課題票のファイルは存在しません。** 一次資料は以下です。各「TBD」は、現時点で該当する成果物が無いことを意味します（推測で内容を補わないこと）。

| 種別 | ファイル/リンク | 概要・用途 |
|------|------------------|------------|
| 要求定義書 | （TBD：未作成） | 役割は README の冒頭＋本書「1.」が代替。正式な要求定義書は無い。 |
| 要件定義書 | `README.md`（特に「マッチング」「設定ファイル」節）/ `src/lib.rs` の `resolve_slot`・`rule_matches` | 実質的な要件・仕様の一次情報。仕様の真実はコードとREADME。 |
| WBS / 進捗 | （TBD：未作成） | git履歴が進捗記録の一部。最新状態は `git status -sb` と `git log -1 --oneline` で確認する。 |
| テスト資産 | `src/lib.rs`（`#[cfg(test)] mod tests`・11件）/ `src/ui.rs`（同・3件） | マッチング・tie-break・曖昧検出・title推定・legacy移行・フォント選定を担保。 |
| 既知課題リスト | （TBD：未作成） | 課題票は無い。気付いた課題は本書「7.」のルールに従い記録・共有する。 |
| 開発・運用コマンド | `justfile` | `doctor` / `build` / `install-local` / `check` / `verify-local` / `shortcuts` / `workflow` 等の正規手順。 |
| ビルド設定・依存 | `Cargo.toml` | 依存クレートとバイナリ定義。`eframe` の feature（`glow` / `x11` / `default_fonts`）に注意。 |

## 4. タスク境界（任せること / 任せないこと）
### 任せるタスク
- `src/lib.rs` / `src/ui.rs` の機能追加・バグ修正（マッチング改善、GUIパネル改善、CLIサブコマンド追加 等）。
- 単体テストの追加・拡充（特に `resolve_slot` のエッジケース、`suggest_title_contains`、`split_prefix_fields`、legacy移行）。
- `README.md` / 本書 / `justfile` のドキュメント・コマンド整備。
- `cargo fmt` / `clippy` 指摘の解消、Rustイディオムへのリファクタ（挙動を変えない範囲）。
- 隔離設定（`XDG_CONFIG_HOME` 切替）でのスモーク確認。

### 任せないタスク（事前承認・確認が必要）
- **ユーザーの本番設定（`~/.config/window-jump/config.json` / `state.json`）の書き換え・削除。** 検証は必ず `XDG_CONFIG_HOME` を一時ディレクトリへ向けて行う。
- **`sudo` を伴う操作・システムへの `apt install` 等。** 必要なら理由と具体コマンドを提示し、ユーザーに実行を依頼する（このプロジェクトでは sudo は人間が実行する運用）。
- **2.「クリティカルな要求・制約」に抵触する変更**（X11前提の放棄、window-idマッチングへの変更、曖昧時に強制前面化する変更、保存のatomic性/ロックの除去、スキーマ非互換変更 等）。やむを得ない場合は必ず事前合意。
- **`Cargo.toml` / `Cargo.lock` の依存バージョン変更や git操作（commit/push/PR）** をユーザーの明示指示なしに行うこと。
- **外部送信（リポジトリ内容・ログ・設定の外部サービスへのアップロード）** を無断で行うこと。

## 5. インタラクション方針
- **回答スタイル:** ビジネス敬語。見出し＋箇条書きを基本とし、コマンドやパスはコードブロック／インラインコードで明示。`file_path:line` 形式で参照すると親切。
- **回答手順:** 「前提（確認した事実）→ 論点 → 提案／実行 → 結果」の順。実行系は、何を・なぜ・どう確認したかをセットで示す。
- **禁止事項・注意:**
  - 未確定事項（TBD）を断定しない。資料が無い箇所は「無い」と明言する。
  - コード／READMEの記述と食い違う「思い込み」を述べない。必ず原典で裏取りする。
  - 破壊的・不可逆操作（設定削除、push、外部送信）は事前確認。
- **秘匿情報の扱い:** 本リポジトリに機密情報は想定されないが、ユーザーのホーム配下の設定ファイル内容・ローカルパス・メールアドレス等を不要に外部へ出力・送信しない。ログや設定の貼り付け時は最小限に。

## 6. 試行タスク（オンボーディング演習）
> 理解度確認用。いずれも本番設定を汚さず、短時間で検証可能なもの。結果を簡潔に報告すること。

1. **環境・品質ゲートの再現:** `just doctor` を実行し、X11セッションと `wmctrl`/`xdotool`/`cargo` の解決状況を報告。続いて `just check` を流し、14件のテストが緑であることを確認・報告する。
2. **マッチング仕様の説明:** `src/lib.rs` の `resolve_slot` を読み、「複数候補が残ったとき、どの補助情報をどの順で使って一意化し、それでも決まらなければどうなるか」を、対応するテスト名（`resolve_slot_uses_last_window_id_as_tie_breaker` 等）を引用して説明する。
3. **隔離スモーク:** 一時ディレクトリを使って本番に影響を与えずに動作確認する。例:
   ```bash
   rm -rf /tmp/wj-test && \
   XDG_CONFIG_HOME=/tmp/wj-test cargo run --bin window-jump -- list-slots
   ```
   出力（未登録時のメッセージ）と、`/tmp/wj-test/window-jump/` に何が生成されるかを報告する。
4. **（任意・発展）テスト追加の素振り:** `suggest_title_contains` か `strip_common_date_suffix` に対し、レッド→グリーンで1ケース追加し、`just check` が通ることを示す（挙動は変えない）。

## 7. 運用ルール・変更管理
- **ドキュメント更新時の記載ルール:** 仕様に関わる変更を入れたら、`README.md`（ユーザー向け）と本書（エージェント向け）の該当箇所を同一変更内で更新する。本書冒頭の「最終更新」も更新する。
- **TBDの扱い:** 未作成資料は「TBD：未作成」と明示し、内容を勝手に創作しない。必要性が生じたら責任者に確認のうえ新規作成する。
- **レビュー/承認フロー:** `commit` / `push` / PR作成・ブランチ運用・依存更新はユーザーの明示指示があってから行う。`main` 直コミットは避け、作業はブランチを切る。コミットメッセージ末尾の Co-Authored-By 規約に従う。
- **その他の運用ルール:**
  - マージ前に必ず `just check`（fmt-check / clippy -D warnings / test）と `just verify-local`（リリースビルド＋隔離スモークまで）を緑にする。
  - 検証は `XDG_CONFIG_HOME` 隔離で行い、本番設定を触らない。
  - 生成物（`target/`・`temp/`）はコミットしない（`.gitignore` 済み）。

---

### 付録: 参考情報
- **主要リポジトリ/ディレクトリ:**
  - リポジトリルート: `/home/inaho/Project/window-jump-config-gui`
  - `src/lib.rs` … 本体（CLI/GUI/マッチング/永続化、約2390行）
  - `src/ui.rs` … 日本語CJKフォント自動検出
  - `src/bin/` … `window-jump.rs`（CLI）/ `window-jump-config-gui.rs`（GUI）のエントリ
  - 設定の既定保存先（実行時に生成）: `~/.config/window-jump/config.json` / `state.json`
- **代表的なコマンド:**
  ```bash
  # 必須ツール・セッション確認
  just doctor

  # 品質ゲート（fmt-check + clippy -D warnings + test）
  just check

  # リリースビルド（rustc 1.92+ を確認してから実行）
  cargo --version
  rustc --version
  cargo build --release --bins

  # ローカル一括検証（fmt/clippy/test/release/version/隔離スモーク）
  just verify-local

  # GNOME Custom Shortcuts 用コマンドの出力
  cargo run --bin window-jump -- shortcut-specs
  # または just shortcuts（リリースバイナリ生成後）

  # 隔離設定での試用（本番を汚さない）
  XDG_CONFIG_HOME=/tmp/wj-test cargo run --bin window-jump -- list-slots
  ```
- **依存ライブラリ（`Cargo.toml`）:**
  - `anyhow`（エラー）, `clap`（CLI, derive）, `serde` / `serde_json`（設定の直列化）
  - `eframe` 0.34（`default_fonts` / `glow` / `x11`、`default-features = false`）
  - **外部実行依存（apt）:** `wmctrl`, `xdotool`（実行時必須）。`libxcb-shape0-dev`, `libxcb-xfixes0-dev`（eframeビルド時必須）。いずれも導入済み。
  - **ツールチェーン:** rustc/cargo **1.92以上**が必須（`eframe`/`egui` 0.34.1 系の要求）。古いツールチェーンを拾う場合は `rustup update stable` 後に `rustup run stable cargo ...` を使う。
  - 環境変数: `WINDOW_JUMP_CLI`（ショートカットに出すCLIパスの上書き）, `XDG_CONFIG_HOME`（設定保存先の上書き）。
- **連絡先/責任者:**
  - リポジトリオーナー / git author: `yuki-inaho`（`yoshikawa@inaho.co`）
  - 上記以外の連絡先・体制は TBD（未定義）。判断に迷う事項はオーナーに確認すること。

> ※本テンプレートは必要に応じて拡張・縮退して構いません。記入済みドキュメントはバージョン管理してください（更新時は冒頭の「最終更新」を必ず更新）。
