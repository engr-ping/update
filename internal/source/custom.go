package source

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"strings"

	"update/internal/config"
	"update/internal/transport"
	"update/internal/version"
)

const maxFeedBytes = 32 << 20

// customSource reads a release feed over HTTP. The feed may be either a
// single release object or an array of release objects (newest first).
type customSource struct {
	cfg    *config.CustomConfig
	client *transport.Client
}

func (s *customSource) fetchFeed(ctx context.Context) ([]*Release, error) {
	resp, err := s.client.Do(ctx, s.cfg.VersionsURL)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	data, err := io.ReadAll(io.LimitReader(resp.Body, maxFeedBytes))
	if err != nil {
		return nil, transport.NewError(transport.KindSource, "read feed: %v", err)
	}
	data = bytes.TrimSpace(data)
	if len(data) == 0 {
		return nil, transport.NewError(transport.KindSource, "empty feed from %s", s.cfg.VersionsURL)
	}

	switch data[0] {
	case '[':
		var list []*Release
		if err := json.Unmarshal(data, &list); err != nil {
			return nil, transport.NewError(transport.KindSource, "decode feed list: %v", err)
		}
		return list, nil
	case '{':
		var one Release
		if err := json.Unmarshal(data, &one); err != nil {
			return nil, transport.NewError(transport.KindSource, "decode feed: %v", err)
		}
		return []*Release{&one}, nil
	default:
		return nil, transport.NewError(transport.KindSource, "feed must be a JSON object or array")
	}
}

// Latest returns the highest version in the feed.
func (s *customSource) Latest(ctx context.Context) (*Release, error) {
	rels, err := s.fetchFeed(ctx)
	if err != nil {
		return nil, err
	}
	best := firstNonNil(rels)
	if best == nil {
		return nil, transport.NewError(transport.KindSource, "feed from %s has no releases", s.cfg.VersionsURL)
	}
	for _, r := range rels {
		if r == nil {
			continue
		}
		if version.Compare(r.Version, best.Version) > 0 {
			best = r
		}
	}
	s.applyTemplate(best)
	return best, nil
}

// List returns the feed in order, capped at limit entries.
func (s *customSource) List(ctx context.Context, limit int) ([]*Release, error) {
	if limit <= 0 {
		limit = 10
	}
	rels, err := s.fetchFeed(ctx)
	if err != nil {
		return nil, err
	}
	out := make([]*Release, 0, len(rels))
	for _, r := range rels {
		if r == nil {
			continue
		}
		s.applyTemplate(r)
		out = append(out, r)
		if len(out) >= limit {
			break
		}
	}
	return out, nil
}

// applyTemplate fills asset URLs from download_url_template when an asset
// has no explicit URL.
func (s *customSource) applyTemplate(r *Release) {
	if s.cfg.DownloadURLTemplate == "" {
		return
	}
	replacer := strings.NewReplacer(
		"{version}", r.Version,
		"{tag_name}", r.TagName,
	)
	for i := range r.Assets {
		if r.Assets[i].URL == "" {
			r.Assets[i].URL = replacer.Replace(strings.ReplaceAll(s.cfg.DownloadURLTemplate, "{asset}", r.Assets[i].Name))
		}
	}
}

func firstNonNil(rels []*Release) *Release {
	for _, r := range rels {
		if r != nil {
			return r
		}
	}
	return nil
}
