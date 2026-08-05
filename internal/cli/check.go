package cli

import (
	"context"
	"fmt"
	"io"

	"update/internal/source"
	"update/internal/version"
)

func runCheck(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	fs := newFlagSet("check", stderr)
	cfgPath := fs.String("config", "", "config file path")
	current := fs.String("current-version", "", "current product version (overrides config)")
	platform := fs.String("platform", "", "target platform as os/arch (default: host)")
	username := fs.String("username", "", "runtime username for authenticated sources (e.g. GUI login)")
	password := fs.String("password", "", "runtime password/token for authenticated sources")
	fs.Bool("json", false, "output JSON (default, accepted for compat)")
	if err := fs.Parse(args); err != nil {
		return 2
	}
	if fs.NArg() > 0 {
		fmt.Fprintf(stderr, "update: unexpected arguments: %v\n", fs.Args())
		return 2
	}

	cfg, code := loadConfig(*cfgPath, stderr)
	if code != 0 {
		return code
	}
	applyCredentials(cfg, *username, *password)
	if *current != "" {
		cfg.Product.CurrentVersion = *current
	}
	osName, arch, err := parsePlatform(*platform)
	if err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return 2
	}

	src, err := source.New(cfg, clientFromConfig(cfg))
	if err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return 2
	}
	rel, err := src.Latest(ctx)
	if err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return 3
	}
	filterAssets(rel, osName, arch, cfg.Product.AssetFilter)

	updateAvailable := cfg.Product.CurrentVersion != "" &&
		version.Compare(rel.Version, cfg.Product.CurrentVersion) > 0

	out := struct {
		Schema          int             `json:"schema"`
		CurrentVersion  string          `json:"current_version"`
		LatestVersion   string          `json:"latest_version"`
		UpdateAvailable bool            `json:"update_available"`
		Release         *source.Release `json:"release"`
	}{
		Schema:          SchemaVersion,
		CurrentVersion:  cfg.Product.CurrentVersion,
		LatestVersion:   rel.Version,
		UpdateAvailable: updateAvailable,
		Release:         rel,
	}
	return writeJSON(stdout, out, stderr)
}
