default:
    @just --list

build:
    cargo build --release

test:
    cargo test --workspace --all-targets

lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

install:
    cargo install --path crates/lgtm-cli

setup-git:
    @echo "# add these to your global git config:"
    @echo ""
    @echo "git config --global diff.tool lgtm"
    @echo "git config --global difftool.lgtm.cmd 'lgtm \"\\$LOCAL\" \"\\$REMOTE\"'"
    @echo ""
    @echo "git config --global merge.tool lgtm"
    @echo "git config --global mergetool.lgtm.cmd 'lgtm --merge \"\\$LOCAL\" \"\\$BASE\" \"\\$REMOTE\" --output \"\\$MERGED\"'"
    @echo "git config --global mergetool.lgtm.trustExitCode true"
