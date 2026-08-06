GO      ?= go
BINARY  := update
PKG     := update/internal/versioninfo
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
COMMIT  := $(shell git rev-parse --short HEAD 2>/dev/null || echo none)
DATE    := $(shell date -u +%Y-%m-%dT%H:%M:%SZ)
LDFLAGS := -s -w -X '$(PKG).Version=$(VERSION)' -X '$(PKG).Commit=$(COMMIT)' -X '$(PKG).Date=$(DATE)'

.PHONY: all build server test vet fmt dist dist-server dist-windows-386 lib clean clean-dist

all: build

## build 构建当前平台二进制 -> bin/update
build:
	CGO_ENABLED=0 $(GO) build -trimpath -ldflags '$(LDFLAGS)' -o bin/$(BINARY) ./cmd/update

## server 构建分发服务器 -> bin/updateserver
server:
	CGO_ENABLED=0 $(GO) build -trimpath -o bin/updateserver ./server

## lib 构建 C ABI 共享库（当前平台）-> dist/libupdate.{so,dylib,dll} + dist/libupdate.h
## 注意：c-shared 必须 CGO_ENABLED=1，且只能针对本机工具链（Linux gcc / macOS clang / Windows mingw）
lib:
	@os=$$($(GO) env GOOS); ext="so"; \
	if [ "$$os" = "darwin" ]; then ext="dylib"; fi; \
	if [ "$$os" = "windows" ]; then ext="dll"; fi; \
	echo "build libupdate.$$ext ($$os)"; \
	CGO_ENABLED=1 $(GO) build -trimpath -buildmode=c-shared \
		-ldflags '$(LDFLAGS)' -o dist/libupdate.$$ext ./cmd/updatelib

## test 运行全部测试
test:
	CGO_ENABLED=0 $(GO) test -count=1 ./...

## vet go vet 静态检查
vet:
	$(GO) vet ./...

## fmt 格式化代码并展示差异
fmt:
	$(GO) fmt ./...

## dist 交叉编译三平台 6 种产物 -> dist/
dist: clean-dist
	@set -e; \
	for t in "linux amd64" "linux arm64" "windows amd64" "windows arm64" "darwin amd64" "darwin arm64"; do \
		set -- $$t; os=$$1; arch=$$2; ext=""; \
		if [ "$$os" = "windows" ]; then ext=".exe"; fi; \
		name="$(BINARY)-$(VERSION)-$$os-$$arch$$ext"; \
		echo "build $$name"; \
		GOOS=$$os GOARCH=$$arch CGO_ENABLED=0 $(GO) build -trimpath -ldflags '$(LDFLAGS)' -o dist/$$name ./cmd/update; \
	done

## dist-server 交叉编译三平台 updateserver -> dist/
dist-server:
	@set -e; \
	for t in "linux amd64" "linux arm64" "windows amd64" "windows arm64" "darwin amd64" "darwin arm64"; do \
		set -- $$t; os=$$1; arch=$$2; ext=""; \
		if [ "$$os" = "windows" ]; then ext=".exe"; fi; \
		name="updateserver-$(VERSION)-$$os-$$arch$$ext"; \
		echo "build $$name"; \
		GOOS=$$os GOARCH=$$arch CGO_ENABLED=0 $(GO) build -trimpath -o dist/$$name ./server; \
	done

## dist-windows-386 交叉编译 Windows x86(32位) CLI + C ABI 库 + 服务器 -> dist/
## 注意：DLL 是 c-shared 构建，需要 32 位 mingw 工具链（Linux: apt install gcc-mingw-w64-i686）
dist-windows-386:
	@echo "build update-$(VERSION)-windows-386.exe"; \
	GOOS=windows GOARCH=386 CGO_ENABLED=0 $(GO) build -trimpath -ldflags '$(LDFLAGS)' \
		-o dist/update-$(VERSION)-windows-386.exe ./cmd/update; \
	echo "build updateserver-$(VERSION)-windows-386.exe"; \
	GOOS=windows GOARCH=386 CGO_ENABLED=0 $(GO) build -trimpath \
		-o dist/updateserver-$(VERSION)-windows-386.exe ./server; \
	echo "build libupdate-windows-386.dll"; \
	GOOS=windows GOARCH=386 CGO_ENABLED=1 CC=$${CC386:-i686-w64-mingw32-gcc} \
		$(GO) build -trimpath -buildmode=c-shared -ldflags '$(LDFLAGS)' \
		-o dist/libupdate-windows-386.dll ./cmd/updatelib

clean: clean-dist
	rm -rf bin

clean-dist:
	rm -rf dist
