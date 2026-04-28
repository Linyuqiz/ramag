# Ramag — 常用任务封装
#
# 直接 `make` 看可用 target；`make help` 同样。
# 默认 target 是 help，避免误触发耗时构建。

# === 配置 ============================================================
BIN_NAME    := ramag
APP_NAME    := Ramag
RELEASE_DIR := target/release
DEBUG_DIR   := target/debug
APP_OUT     := target/$(APP_NAME).app

# 让 cargo 输出更易读
CARGO       := cargo
CARGO_FLAGS :=

# 让所有 target 默认是 phony（不对应文件）
.PHONY: help \
        run dev release \
        check fmt fmt-check clippy test \
        dmg dmg-x86 dmg-arm64 dmg-universal \
        clean clean-all \
        deps-update lock-refresh

# === 默认 ============================================================
.DEFAULT_GOAL := help

help:
	@printf "\033[1mRamag — 常用命令\033[0m\n\n"
	@printf "  \033[36m开发\033[0m\n"
	@printf "    make run            cargo run（debug，快编译，慢运行）\n"
	@printf "    make dev            cargo run（同上，dev profile 显式版本）\n"
	@printf "    make release        cargo run --release（慢编译，快运行；首次 ~2-3 分钟）\n"
	@printf "\n  \033[36m检查\033[0m\n"
	@printf "    make check          cargo check（最快的语法/类型检查）\n"
	@printf "    make fmt            cargo fmt 格式化全部代码\n"
	@printf "    make fmt-check      cargo fmt --check（CI 用：不改文件，只校验）\n"
	@printf "    make clippy         cargo clippy --all-targets -- -D warnings\n"
	@printf "    make test           cargo test\n"
	@printf "\n  \033[36m打包\033[0m\n"
	@printf "    make dmg            当前架构（native）：svg → icns → cargo build → Ramag.app → Ramag.dmg\n"
	@printf "    make dmg-x86        交叉编译 Intel mac：Ramag-x86_64.dmg\n"
	@printf "    make dmg-arm64      交叉编译 Apple Silicon：Ramag-arm64.dmg\n"
	@printf "    make dmg-universal  Intel + Apple Silicon 通用二进制（编译时间约 2 倍）\n"
	@printf "\n  \033[36m清理\033[0m\n"
	@printf "    make clean          cargo clean（删 target/）\n"
	@printf "    make clean-all      clean + 删 Ramag.app + 生成的 icns\n"
	@printf "\n  \033[36m依赖\033[0m\n"
	@printf "    make deps-update    cargo update（升级 Cargo.lock 到允许的最新版）\n"
	@printf "    make lock-refresh   删除 Cargo.lock 后重新解析（git 依赖会拉最新 master）\n"

# === 开发 ============================================================
develop:
	$(CARGO) run -p ramag-bin $(CARGO_FLAGS)

release:
	$(CARGO) run --release -p ramag-bin $(CARGO_FLAGS)

# === 检查 ============================================================
check:
	$(CARGO) check --all-targets

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

test:
	$(CARGO) test --all

# === 打包 ============================================================
# 一条龙：build-dmg.sh 内部跑 svg→icns、cargo build、组装 .app、打 dmg 全流程。
# 交叉编译目标若未安装 rustup target，脚本会自动 rustup target add。
dmg:
	./scripts/build-dmg.sh

dmg-x86:
	./scripts/build-dmg.sh --target=x86_64

dmg-arm64:
	./scripts/build-dmg.sh --target=arm64

dmg-universal:
	./scripts/build-dmg.sh --target=universal

# === 清理 ============================================================
clean:
	$(CARGO) clean

clean-all: clean
	rm -rf target/Ramag*.app
	rm -rf target/dmg-staging*
	rm -f target/Ramag*.dmg
	rm -f scripts/icons/ramag.icns

# === 依赖 ============================================================
deps-update:
	$(CARGO) update

lock-refresh:
	rm -f Cargo.lock
	$(CARGO) check
