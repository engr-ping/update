// Package cli implements the update command-line interface.
//
// Contract (see docs/design.md):
//   - stdout carries protocol JSON only
//   - logs and errors go to stderr
//   - exit codes: 0 success, 2 config/usage error, 3 source error,
//     4 download error
package cli

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"regexp"
	"runtime"
	"strings"

	"update/internal/config"
	"update/internal/source"
	"update/internal/transport"
	"update/internal/version"
	"update/internal/versioninfo"
)

// SchemaVersion is the JSON protocol schema version emitted on stdout.
const SchemaVersion = 1

// Run executes the update CLI and returns the process exit code.
func Run(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	if len(args) == 0 {
		usage(stderr)
		return 0
	}
	switch args[0] {
	case "help", "--help", "-h":
		usage(stdout)
		return 0
	case "check":
		return runCheck(ctx, args[1:], stdout, stderr)
	case "download":
		return runDownload(ctx, args[1:], stdout, stderr)
	case "list":
		return runList(ctx, args[1:], stdout, stderr)
	case "version":
		fmt.Fprintln(stdout, versioninfo.String())
		return 0
	default:
		fmt.Fprintf(stderr, "update: unknown command %q\n\n", args[0])
		usage(stderr)
		return 2
	}
}

func usage(w io.Writer) {
	fmt.Fprint(w, `update - language-agnostic software update module

Usage:
  update check    [--config FILE] [--current-version X] [--platform os/arch] [--username U] [--password P]
  update download [--config FILE] --version X [--asset NAME] [--out PATH] [--platform os/arch] [--username U] [--password P]
  update list     [--config FILE] [--limit N] [--platform os/arch] [--username U] [--password P]
  update version
  update help

Credentials: sources authenticate via env vars named in config (token_env /
username_env). For runtime login (e.g. GUI), pass --username/--password to
override them; username implies Basic auth, password alone means Bearer.

Exit codes:
  0  success
  2  config or usage error
  3  source error (network, HTTP, auth, parse)
  4  download error (checksum mismatch, write failure)
`)
}

func loadConfig(flagPath string, stderr io.Writer) (*config.Config, int) {
	path := flagPath
	if path == "" {
		path = os.Getenv("UPDATE_CONFIG")
	}
	if path == "" {
		fmt.Fprintln(stderr, "update: no config file: use --config FILE or UPDATE_CONFIG env var")
		return nil, 2
	}
	cfg, err := config.Load(path, os.Getenv)
	if err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return nil, 2
	}
	return cfg, 0
}

// clientFromConfig wires config credentials into the transport layer.
func clientFromConfig(cfg *config.Config) *transport.Client {
	var auth *transport.Auth
	var headers map[string]string
	switch cfg.Source.Type {
	case "github-tag":
		g := cfg.Source.GitHub
		switch {
		case g.Username != "":
			auth = &transport.Auth{Type: "basic", Username: g.Username, Token: g.Token}
		case g.Token != "":
			auth = &transport.Auth{Type: "bearer", Token: g.Token}
		}
	case "custom":
		headers = cfg.Source.Custom.Headers
		if a := cfg.Source.Custom.Auth; a != nil {
			auth = &transport.Auth{Type: a.Type, Token: a.Token, Username: a.Username}
		}
	}
	return transport.New(transport.Options{Auth: auth, Headers: headers})
}

// applyCredentials overrides the config's credentials with runtime
// --username/--password values (e.g. from a GUI login dialog). A username
// implies Basic auth; username empty but password set means Bearer (token).
func applyCredentials(cfg *config.Config, username, password string) {
	if username == "" && password == "" {
		return
	}
	switch cfg.Source.Type {
	case "github-tag":
		g := cfg.Source.GitHub
		g.Username = username
		if password != "" {
			g.Token = password
		}
	case "custom":
		cu := cfg.Source.Custom
		if cu.Auth == nil {
			cu.Auth = &config.AuthConfig{Type: "basic"}
		}
		if username != "" {
			cu.Auth.Type = "basic"
			cu.Auth.Username = username
		}
		if password != "" {
			cu.Auth.Token = password
		}
	}
}

// parsePlatform resolves the --platform flag to os/arch. Empty string means
// the host platform; "all" means no platform filtering.
func parsePlatform(s string) (osName, arch string, err error) {
	switch s {
	case "":
		return runtime.GOOS, runtime.GOARCH, nil
	case "all":
		return "", "", nil
	}
	parts := strings.Split(s, "/")
	if len(parts) != 2 || parts[0] == "" || parts[1] == "" {
		return "", "", fmt.Errorf("invalid --platform %q (want os/arch)", s)
	}
	return parts[0], parts[1], nil
}

// filterAssets keeps only assets that match the target platform (or the
// configured asset_filter regex, which takes precedence when set).
func filterAssets(rel *source.Release, osName, arch, filter string) {
	if osName == "" || arch == "" {
		return
	}
	var re *regexp.Regexp
	if filter != "" {
		re, _ = regexp.Compile(filter)
	}
	kept := rel.Assets[:0]
	for _, a := range rel.Assets {
		if re != nil {
			if re.MatchString(a.Name) {
				kept = append(kept, a)
			}
			continue
		}
		if version.MatchAsset(a.Name, osName, arch) {
			kept = append(kept, a)
		}
	}
	rel.Assets = kept
}

func writeJSON(w io.Writer, v interface{}, stderr io.Writer) int {
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		fmt.Fprintf(stderr, "update: encode output: %v\n", err)
		return 1
	}
	return 0
}

func newFlagSet(name string, stderr io.Writer) *flag.FlagSet {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	fs.SetOutput(stderr)
	return fs
}
