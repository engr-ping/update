GO      ?= go
BINARY  := update
PKG     := update/internal/versioninfo
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
COMMIT  := $(shell git rev-parse --short HEAD 2>/dev/null || echo none)
DATE    := $(shell date -u +%Y-%m-%dT%H:%M:%SZ)
LDFLAGS := -s -w -X '$(PKG).Version=$(VERSION)' -X '$(PKG).Commit=$(COMMIT)' -X '$(PKG).Date=$(DATE)'

.PHONY: all build test vet fmt dist lib clean

all: build

## build 构建当前平台二进制 -> bin/update
build:
	CGO_ENABLED=0 $(GO) build -trimpath -ldflags '$(LDFLAGS)' -o bin/$(BINARY) ./cmd/update

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

clean: clean-dist
	rm -rf bin

clean-dist:
	rm -rf dist
