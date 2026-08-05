package cli

import (
	"context"
	"fmt"
	"io"

	"update/internal/source"
)

func runDownload(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	fs := newFlagSet("download", stderr)
	cfgPath := fs.String("config", "", "config file path")
	ver := fs.String("version", "", "version to download, or \"latest\" (required)")
	asset := fs.String("asset", "", "exact asset name to download")
	out := fs.String("out", "", "output path (default: asset name in current dir)")
	platform := fs.String("platform", "", "target platform as os/arch (default: host)")
	username := fs.String("username", "", "runtime username for authenticated sources (e.g. GUI login)")
	password := fs.String("password", "", "runtime password/token for authenticated sources")
	if err := fs.Parse(args); err != nil {
		return 2
	}
	if fs.NArg() > 0 {
		fmt.Fprintf(stderr, "update: unexpected arguments: %v\n", fs.Args())
		return 2
	}
	if *ver == "" {
		fmt.Fprintln(stderr, "update: --version is required")
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

	client := clientFromConfig(cfg)
	src, err := source.New(cfg, client)
	if err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return 2
	}

	var rel *source.Release
	if *ver == "latest" {
		rel, err = src.Latest(ctx)
	} else {
		rel, err = findRelease(ctx, src, *ver)
	}
	if err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return 3
	}
	if rel == nil {
		fmt.Fprintf(stderr, "update: version %q not found\n", *ver)
		return 3
	}
	filterAssets(rel, osName, arch, cfg.Product.AssetFilter)

	target, err := pickAsset(rel, *asset)
	if err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return 4
	}
	if target.URL == "" {
		fmt.Fprintf(stderr, "update: asset %q has no download url\n", target.Name)
		return 4
	}

	dest := *out
	if dest == "" {
		dest = target.Name
	}
	if err := client.Download(ctx, target.URL, dest, target.SHA256); err != nil {
		fmt.Fprintf(stderr, "update: %v\n", err)
		return 4
	}

	result := struct {
		Schema  int    `json:"schema"`
		Version string `json:"version"`
		File    string `json:"file"`
	}{
		Schema:  SchemaVersion,
		Version: rel.Version,
		File:    dest,
	}
	return writeJSON(stdout, result, stderr)
}

// findRelease locates a release by exact version or tag name.
func findRelease(ctx context.Context, src source.Source, ver string) (*source.Release, error) {
	rels, err := src.List(ctx, 100)
	if err != nil {
		return nil, err
	}
	for _, r := range rels {
		if r == nil {
			continue
		}
		if r.TagName == ver || r.Version == ver {
			return r, nil
		}
	}
	return nil, nil
}

// pickAsset selects the asset to download. When name is empty the first
// matching (already platform-filtered) asset is used.
func pickAsset(rel *source.Release, name string) (*source.Asset, error) {
	for i := range rel.Assets {
		if name == "" || rel.Assets[i].Name == name {
			return &rel.Assets[i], nil
		}
	}
	if name != "" {
		return nil, fmt.Errorf("asset %q not found in version %s", name, rel.Version)
	}
	return nil, fmt.Errorf("no asset available for this platform in version %s", rel.Version)
}
