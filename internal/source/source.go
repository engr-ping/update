// Package source defines the release-source abstraction and implements the
// GitHub tag/release source and the custom HTTP source.
package source

import (
	"context"
	"fmt"

	"update/internal/config"
	"update/internal/transport"
)

// Release is the unified release model shared by all sources.
type Release struct {
	Version     string  `json:"version"`
	TagName     string  `json:"tag_name,omitempty"`
	PublishedAt string  `json:"published_at,omitempty"`
	Name        string  `json:"name,omitempty"`
	Notes       string  `json:"notes,omitempty"`
	Checksum    string  `json:"checksum,omitempty"`
	Assets      []Asset `json:"assets"`
}

// Asset is a downloadable file attached to a release.
type Asset struct {
	Name   string `json:"name"`
	URL    string `json:"url"`
	Size   int64  `json:"size,omitempty"`
	SHA256 string `json:"sha256,omitempty"`
}

// Source fetches release information from an upstream.
type Source interface {
	// Latest returns the newest release.
	Latest(ctx context.Context) (*Release, error)
	// List returns releases, newest first, at most limit entries.
	List(ctx context.Context, limit int) ([]*Release, error)
}

// New constructs a Source for the given configuration.
func New(cfg *config.Config, client *transport.Client) (Source, error) {
	switch cfg.Source.Type {
	case "github-tag":
		return &githubSource{cfg: cfg.Source.GitHub, client: client}, nil
	case "custom":
		return &customSource{cfg: cfg.Source.Custom, client: client}, nil
	default:
		return nil, fmt.Errorf("unsupported source type %q", cfg.Source.Type)
	}
}
