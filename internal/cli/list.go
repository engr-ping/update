package cli

import (
	"context"
	"fmt"
	"io"

	"update/internal/source"
)

func runList(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	fs := newFlagSet("list", stderr)
	cfgPath := fs.String("config", "", "config file path")
	limit := fs.Int("limit", 10, "max number of versions")
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
	if *limit < 1 {
		fmt.Fprintln(stderr, "update: --limit must be >= 1")
		return 2
	}

	cfg, code := loadConfig(*cfgPath, stderr)
	if code != 0 {
		return code
	}
	applyCredentials(cfg, *username, *password)
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
	rels, err := src.List(ctx, *limit)
	if err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return 3
	}
	for _, r := range rels {
		if r != nil {
			filterAssets(r, osName, arch, cfg.Product.AssetFilter)
		}
	}

	out := struct {
		Schema   int               `json:"schema"`
		Versions []*source.Release `json:"versions"`
	}{
		Schema:   SchemaVersion,
		Versions: rels,
	}
	return writeJSON(stdout, out, stderr)
}
