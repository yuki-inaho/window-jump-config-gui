# window-jump

Ubuntu GNOME の X11 セッション向けのウィンドウジャンプツールです。

`window-jump-config-gui` で slot 1..9 にウィンドウの再発見ルールを登録し、`window-jump activate N` で対象ウィンドウを前面化します。保存するのは window id ではなく、主に `WM_CLASS` と `title_contains` です。

## 関連ドキュメント

通常利用はこの README だけで足ります。詳しく確認したい場合は次も見てください。

| 名前 | リンク | 立ち位置 |
| --- | --- | --- |
| LLMオンボーディングサマリー | [docs/ONBOARDING.md](docs/ONBOARDING.md) | 新規LLMエージェント向けの確認順序・重要制約・運用メモ |

## 前提

```bash
echo "$XDG_SESSION_TYPE"
command -v wmctrl
command -v xdotool
```

`XDG_SESSION_TYPE` は `x11` を想定しています。`wmctrl` と `xdotool` がない場合は導入します。

```bash
sudo apt update
sudo apt install -y wmctrl xdotool
```

ソースからGUIをビルドする環境で xcb 系の開発パッケージが足りない場合は、次も入れます。

```bash
sudo apt install -y libxcb-shape0-dev libxcb-xfixes0-dev
```

Rust と `cargo` も必要です。`justfile` を使う場合は `just` も導入してください。

```bash
cargo install just --locked
```

## ビルドと配置

`justfile` を使う場合:

```bash
just doctor
just build
just install-local
```

生の `cargo` コマンドを使う場合:

```bash
cargo build --release --bins
mkdir -p "$HOME/.local/bin"
install -m 0755 target/release/window-jump "$HOME/.local/bin/window-jump"
install -m 0755 target/release/window-jump-config-gui "$HOME/.local/bin/window-jump-config-gui"
```

PATH に入っているか確認します。

```bash
command -v window-jump
command -v window-jump-config-gui
```

## GUI で登録する

```bash
window-jump-config-gui
```

1. 左の `Slots 1..9` から slot を選びます。
2. `登録モード: クリックで選択` を押します。
3. 登録したいウィンドウをクリックします。
4. `Registration candidate` の内容を確認します。
5. 必要なら `Label` と `Title contains` を調整します。
6. `OK: register to this slot` を押します。
7. `Test activate` で前面化を確認します。

登録候補は `OK: register to this slot` を押すまで保存されません。登録済みルールを編集した場合も、`Save` を押すまで保存されません。

`Window catalog` は初期状態では閉じています。現在の X11 ウィンドウ一覧から選びたい場合だけ、`Window catalog を開く` を押します。

終了は `q` または `Esc` です。テキスト入力中は通常のキー入力として扱われます。

## GNOME Custom Shortcuts

ショートカット設定画面を開きます。

```bash
gnome-control-center keyboard
```

`Keyboard Shortcuts` / `View and Customize Shortcuts` / `Custom Shortcuts` から追加します。

`window-jump` の絶対パスは次で確認します。

```bash
command -v window-jump
```

登録例:

| 項目 | 入力例 |
| --- | --- |
| Name | `Window Jump Slot 1` |
| Command | `<command -v window-jump の出力> activate 1` |
| Shortcut | `Ctrl+Alt+1` |

slot 2 以降も同じです。

```text
Ctrl+Alt+1 -> <window-jump の絶対パス> activate 1
Ctrl+Alt+2 -> <window-jump の絶対パス> activate 2
...
Ctrl+Alt+9 -> <window-jump の絶対パス> activate 9
```

コマンド一覧を出す場合:

```bash
window-jump shortcut-specs
# または
just shortcuts
```

## CLI

よく使うコマンドだけ載せます。全体は `window-jump --help` で確認してください。

```bash
window-jump activate 1
window-jump capture-active 1
window-jump capture-click 1
window-jump clear 1
window-jump list-slots
window-jump list-windows
window-jump inspect-active
window-jump check-fonts
```

通常設定を触らずに試す場合:

```bash
XDG_CONFIG_HOME=/tmp/window-jump-test window-jump-config-gui
XDG_CONFIG_HOME=/tmp/window-jump-test window-jump list-slots
```

## 設定ファイル

既定の保存先:

```text
~/.config/window-jump/config.json
~/.config/window-jump/state.json
```

`config.json` には slot のルールが入ります。

```json
{
  "version": 1,
  "slots": {
    "1": {
      "label": "Firefox Inbox",
      "wm_class": "Navigator.Firefox",
      "title_contains": "Inbox",
      "notes": ""
    }
  }
}
```

`state.json` は前回一致した window id や desktop などの補助情報です。通常は直接編集しません。

## マッチング

`activate` は現在の X11 ウィンドウ一覧から、次の順で対象を探します。

1. 前回の `last_window_id` がまだ存在し、`WM_CLASS` も同じなら、そのウィンドウを使います。
2. それで決まらなければ、`WM_CLASS` が一致するウィンドウを探します。
3. `title_contains` が空でなければ、タイトル部分一致で絞ります。
4. 1 件に決まれば前面化します。
5. 複数残る場合は、候補の中で `last_window_id` や desktop を補助情報として使います。
6. まだ複数残る場合は、誤操作を避けるため失敗します。

`last_window_id` は同じ X11 ウィンドウが生きている間だけ使う実行時ヒントです。アプリ再起動後も同じ対象を示す永続 ID ではありません。

Terminal、Codex、Claude Code などはタブや作業内容でタイトルが変わることがあります。`title_contains` は短く安定した文字列にし、不要なら空にします。

Window Jump が前面化するのは X11 のトップレベルウィンドウです。端末内のタブやペインは切り替えません。

`title_contains` はフルタイトルではなく、短く変わりにくい文字列にします。

```text
悪い例:
some-document_Apr16-2026.md - Editor - Browser

良い例:
some-document
```

## 開発時の確認

```bash
just check
just verify-local
```

`cargo` で直接見る場合:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release --bins
```

## トラブルシュート

| 症状 | 対応 |
| --- | --- |
| Wayland で動かない | X11 専用です。`echo "$XDG_SESSION_TYPE"` を確認します。 |
| `wmctrl` がない | `sudo apt install wmctrl` |
| `xdotool` がない | `sudo apt install xdotool` |
| 対象が見つからない | `window-jump list-windows` と `window-jump list-slots` を確認します。 |
| 候補が複数残る | `Title contains` をもう少し具体的にします。 |
| タイトル変更で見つからない | `Title contains` を短く安定した文字列にします。 |
| GUI が read-only になる | `config.json` または `state.json` の JSON を確認します。 |
| ショートカットから起動しない | GNOME 側の `Command` が絶対パスか確認します。 |
