# justfile — Ubuntu GNOME on X11 向けの開発・運用コマンド定義。
#
# 前提: CWD はリポジトリ直下。

set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just help

help:
    @printf '%s\n' \
      'Window Jump - development and workflow commands' \
      '' \
      'First run:' \
      '  just doctor                         # required tools and session checks' \
      '  just build                          # release build for both binaries' \
      '  just install-local                  # install binaries to ~/.local/bin' \
      '' \
      'Daily development:' \
      '  just check                          # fmt-check + clippy + unit tests' \
      '  just quality                        # same as check for now' \
      '  just fmt                            # apply rustfmt' \
      '  just run-gui                        # launch config GUI via cargo' \
      '  just run-cli -- --help              # pass args to CLI via cargo' \
      '  just run-release-gui                # launch built GUI binary' \
      '  just run-release-cli -- list-slots  # pass args to built CLI binary' \
      '' \
      'Window Jump workflow:' \
      '  just workflow                       # show normal setup workflow' \
      '  just shortcuts                      # print GNOME Custom Shortcuts commands' \
      '  just verify-local                   # local verification and isolated config smoke' \
      '' \
      'Reference:' \
      '  recipe list: just --list'

doctor:
    @missing=0; \
    printf 'session: XDG_SESSION_TYPE=%s\n' "$${XDG_SESSION_TYPE:-}"; \
    if [ "$${XDG_SESSION_TYPE:-}" = "x11" ]; then \
      printf 'ok: X11 session\n'; \
    else \
      printf 'warning: this tool is X11-only\n' >&2; \
    fi; \
    for cmd in cargo wmctrl xdotool; do \
      if command -v "$cmd" >/dev/null 2>&1; then \
        printf 'ok: %s -> %s\n' "$cmd" "$$(command -v "$cmd")"; \
      else \
        printf 'missing: %s\n' "$cmd" >&2; \
        missing=1; \
      fi; \
    done; \
    if command -v just >/dev/null 2>&1; then \
      printf 'ok: just -> %s\n' "$$(command -v just)"; \
    else \
      printf 'optional missing: just\n' >&2; \
    fi; \
    exit "$missing"

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test

check:
    just fmt-check
    just clippy
    just test

quality:
    just check

build:
    cargo build --release --bins

install-local:
    mkdir -p "$HOME/.local/bin"
    install -m 0755 target/release/window-jump "$HOME/.local/bin/window-jump"
    install -m 0755 target/release/window-jump-config-gui "$HOME/.local/bin/window-jump-config-gui"
    command -v window-jump
    command -v window-jump-config-gui

run-cli *args:
    cargo run --bin window-jump -- {{args}}

run-gui:
    cargo run --bin window-jump-config-gui

run-release-cli *args:
    target/release/window-jump {{args}}

run-release-gui:
    target/release/window-jump-config-gui

shortcuts:
    target/release/window-jump shortcut-specs

workflow:
    @printf '%s\n' \
      'Window Jump normal workflow' \
      '' \
      '1. just doctor' \
      '2. just build' \
      '3. just install-local' \
      '4. window-jump-config-gui' \
      '5. Register slots from active/click/live window candidates' \
      '6. window-jump shortcut-specs' \
      '7. Add commands to GNOME Custom Shortcuts'

verify-local:
    @echo "[verify-local] 1/6 cargo fmt --check"
    cargo fmt --all -- --check
    @echo "[verify-local] 2/6 cargo clippy -D warnings"
    cargo clippy --all-targets --all-features -- -D warnings
    @echo "[verify-local] 3/6 cargo test"
    cargo test
    @echo "[verify-local] 4/6 release build"
    cargo build --release --bins
    @echo "[verify-local] 5/6 version checks"
    target/release/window-jump --version
    target/release/window-jump-config-gui --version
    @echo "[verify-local] 6/6 isolated config smoke"
    rm -rf temp/verify-config
    XDG_CONFIG_HOME="$PWD/temp/verify-config" target/release/window-jump list-slots
