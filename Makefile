# Rust workspace 构建脚本（零第三方依赖：仅 cargo/rustup + 系统链接器）
# 版本注入：build.rs 读取 UPDATE_VERSION/UPDATE_COMMIT/UPDATE_DATE 环境变量（缺省回退 git describe），
# 等价旧 Go 版 ldflags 注入方式。
VERSION ?= $(shell git describe --tags --always 2>/dev/null || echo 1.0.0)
COMMIT  := $(shell git rev-parse --short HEAD 2>/dev/null || echo none)
DATE    := $(shell date -u +%Y-%m-%dT%H:%M:%SZ)
VER_ENV := UPDATE_VERSION=$(VERSION) UPDATE_COMMIT=$(COMMIT) UPDATE_DATE=$(DATE)

# 三平台六种交叉编译 target（每平台 host 编译自身两份产物）
LINUX_TARGETS   := x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
WINDOWS_TARGETS := x86_64-pc-windows-msvc aarch64-pc-windows-msvc
DARWIN_TARGETS  := x86_64-apple-darwin aarch64-apple-darwin
ALL_TARGETS     := $(LINUX_TARGETS) $(WINDOWS_TARGETS) $(DARWIN_TARGETS)

.PHONY: all build server test vet fmt dist dist-server lib clean clean-dist

all: build

## build 构建当前平台客户端二进制 -> bin/update（workspace 全量 release 构建）
build:
	$(VER_ENV) cargo build --release
	mkdir -p bin
	cp target/release/update bin/update

## server 构建分发服务器 -> bin/updateserver
server:
	$(VER_ENV) cargo build --release -p update-server
	mkdir -p bin
	cp target/release/updateserver bin/updateserver

## test 运行全部测试（workspace）
test:
	cargo test --workspace

## vet cargo clippy 静态检查（对应旧 go vet；-D warnings 零容忍）
vet:
	cargo clippy --workspace --all-targets -- -D warnings

## fmt 格式检查（未安装 rustfmt 时提示跳过）
fmt:
	@if cargo fmt --version >/dev/null 2>&1; then cargo fmt --check; else \
		echo "跳过 fmt：未安装 rustfmt（rustup component add rustfmt）"; \
	fi

## dist 客户端三平台 6 种产物 -> dist/，命名 update-$(VERSION)-<os>-<arch>[.exe]
## 每平台 host 编译自身产物；linux arm64 在 linux host 上尝试交叉编译，
## 缺少 rustup target 或交叉链接器（gcc-aarch64-linux-gnu）时提示跳过，不做假成功。
dist: clean-dist
	@set -e; \
	os=$$(uname -s | tr '[:upper:]' '[:lower:]'); \
	case "$$os" in \
	linux|darwin) ;; \
	mingw*|msys*|cygwin*) os=windows ;; \
	*) echo "未知平台 $$os，跳过 dist"; exit 0 ;; \
	esac; \
	for t in $(ALL_TARGETS); do \
		arch=$$(echo "$$t" | sed -e 's/^x86_64/amd64/' -e 's/^aarch64/arm64/'); \
		ext=""; \
		case "$$t" in \
		*-linux-*)   [ "$$os" = "linux" ]   || { echo "跳过 $$t（仅 linux host 可构建）"; continue; }; \
			[ "$$t" != "aarch64-unknown-linux-gnu" ] || { \
				rustup target list --installed 2>/dev/null | grep -qx "$$t" || { echo "跳过 $$t：缺少 rustup target（rustup target add aarch64-unknown-linux-gnu）"; continue; }; \
				command -v aarch64-linux-gnu-gcc >/dev/null 2>&1 || { echo "跳过 $$t：缺少交叉链接器（apt install gcc-aarch64-linux-gnu）"; continue; }; \
				export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc; \
				export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc; \
			};; \
		*-windows-*) [ "$$os" = "windows" ] || { echo "跳过 $$t（仅 windows host 可构建）"; continue; }; \
			ext=".exe";; \
		*-darwin-*)  [ "$$os" = "darwin" ]  || { echo "跳过 $$t（仅 macos host 可构建）"; continue; };; \
		esac; \
		name="update-$(VERSION)-$$os-$$arch$$ext"; \
		echo "build $$name"; \
		$(VER_ENV) cargo build --release --target "$$t" -p update-cli; \
		cp "target/$$t/release/update" "dist/$$name"; \
	done

## dist-server 服务器三平台 6 种产物 -> dist/，命名 updateserver-$(VERSION)-<os>-<arch>[.exe]
## 平台/工具链策略与 dist 相同：缺失工具链提示跳过。
dist-server: clean-dist
	@set -e; \
	os=$$(uname -s | tr '[:upper:]' '[:lower:]'); \
	case "$$os" in \
	linux|darwin) ;; \
	mingw*|msys*|cygwin*) os=windows ;; \
	*) echo "未知平台 $$os，跳过 dist-server"; exit 0 ;; \
	esac; \
	for t in $(ALL_TARGETS); do \
		arch=$$(echo "$$t" | sed -e 's/^x86_64/amd64/' -e 's/^aarch64/arm64/'); \
		ext=""; \
		case "$$t" in \
		*-linux-*)   [ "$$os" = "linux" ]   || { echo "跳过 $$t（仅 linux host 可构建）"; continue; }; \
			[ "$$t" != "aarch64-unknown-linux-gnu" ] || { \
				rustup target list --installed 2>/dev/null | grep -qx "$$t" || { echo "跳过 $$t：缺少 rustup target（rustup target add aarch64-unknown-linux-gnu）"; continue; }; \
				command -v aarch64-linux-gnu-gcc >/dev/null 2>&1 || { echo "跳过 $$t：缺少交叉链接器（apt install gcc-aarch64-linux-gnu）"; continue; }; \
				export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc; \
				export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc; \
			};; \
		*-windows-*) [ "$$os" = "windows" ] || { echo "跳过 $$t（仅 windows host 可构建）"; continue; }; \
			ext=".exe";; \
		*-darwin-*)  [ "$$os" = "darwin" ]  || { echo "跳过 $$t（仅 macos host 可构建）"; continue; };; \
		esac; \
		name="updateserver-$(VERSION)-$$os-$$arch$$ext"; \
		echo "build $$name"; \
		$(VER_ENV) cargo build --release --target "$$t" -p update-server; \
		cp "target/$$t/release/updateserver" "dist/$$name"; \
	done

## lib 构建当前平台 C ABI 共享库 -> dist/libupdate.{so,dylib,dll} + dist/libupdate.h
## 当前平台只能产当前平台的库，其他平台提示跳过；Windows 需要 mingw gcc。
lib: clean-dist
	@mkdir -p dist; \
	set -e; \
	os=$$(uname -s | tr '[:upper:]' '[:lower:]'); \
	case "$$os" in \
	linux) \
		host=$$(rustc -vV | sed -n 's/^host: //p'); ext="so";; \
	darwin) \
		host=$$(rustc -vV | sed -n 's/^host: //p'); ext="dylib";; \
	mingw*|msys*|cygwin*) \
		command -v gcc >/dev/null 2>&1 || { echo "跳过 lib：Windows 需要 mingw gcc"; exit 0; }; \
		host=x86_64-pc-windows-gnu; ext="dll"; \
		rustup target list --installed 2>/dev/null | grep -qx "$$host" || { echo "跳过 lib：缺少 rustup target $$host"; exit 0; };; \
	*) echo "跳过 lib：未知平台 $$os"; exit 0 ;; \
	esac; \
	name="libupdate.$$ext"; \
	echo "build $$name ($$host)"; \
	$(VER_ENV) cargo build --release -p update-lib --target "$$host"; \
	libfile="$$name"; \
	case "$$os" in linux|darwin) libfile="lib$$libfile";; esac; \
	cp "target/$$host/release/$$libfile" "dist/$$name"; \
	cp crates/update-lib/include/libupdate.h dist/libupdate.h

## clean 清理构建缓存与产物 -> cargo clean + 移除 bin/ dist/
clean: clean-dist
	cargo clean
	rm -rf bin

## clean-dist 仅移除 dist/ 产物
clean-dist:
	rm -rf dist