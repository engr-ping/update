package source

import (
	"context"
	"fmt"
	"sort"
	"strings"

	"update/internal/config"
	"update/internal/transport"
	"update/internal/version"
)

// githubSource reads releases or tags from the GitHub REST API.
type githubSource struct {
	cfg    *config.GitHubConfig
	client *transport.Client
}

// githubRelease is the subset of the GitHub releases API object we need.
type githubRelease struct {
	TagName     string        `json:"tag_name"`
	Name        string        `json:"name"`
	PublishedAt string        `json:"published_at"`
	Body        string        `json:"body"`
	Assets      []githubAsset `json:"assets"`
	ZipballURL  string        `json:"zipball_url"`
	TarballURL  string        `json:"tarball_url"`
	Draft       bool          `json:"draft"`
	Prerelease  bool          `json:"prerelease"`
}

type githubAsset struct {
	Name               string `json:"name"`
	BrowserDownloadURL string `json:"browser_download_url"`
	Size               int64  `json:"size"`
}

type githubTag struct {
	Name string `json:"name"`
}

func (s *githubSource) base() string { return strings.TrimRight(s.cfg.APIBaseURL, "/") }

func (s *githubSource) latestURL() string {
	if s.cfg.UseReleases {
		return fmt.Sprintf("%s/repos/%s/%s/releases/latest", s.base(), s.cfg.Owner, s.cfg.Repo)
	}
	return s.tagsURL(100)
}

func (s *githubSource) listURL(limit int) string {
	if s.cfg.UseReleases {
		return fmt.Sprintf("%s/repos/%s/%s/releases?per_page=%d", s.base(), s.cfg.Owner, s.cfg.Repo, limit)
	}
	return s.tagsURL(limit)
}

func (s *githubSource) tagsURL(perPage int) string {
	return fmt.Sprintf("%s/repos/%s/%s/tags?per_page=%d", s.base(), s.cfg.Owner, s.cfg.Repo, perPage)
}

// Latest returns the newest release (or highest tag when use_releases=false).
// If the repo has tags but no releases, it falls back to tags.
func (s *githubSource) Latest(ctx context.Context) (*Release, error) {
	if s.cfg.UseReleases {
		rel := &githubRelease{}
		if err := s.client.GetJSON(ctx, s.latestURL(), rel); err != nil {
			if te, ok := err.(*transport.Error); ok && te.StatusCode == 404 {
				return s.latestFromTags(ctx)
			}
			return nil, err
		}
		if rel.Draft {
			return nil, transport.NewError(transport.KindSource, "latest release %q is a draft", rel.TagName)
		}
		r := s.fromGithubRelease(rel)
		return &r, nil
	}
	return s.latestFromTags(ctx)
}

func (s *githubSource) latestFromTags(ctx context.Context) (*Release, error) {
	tags, err := s.fetchTags(ctx, 100)
	if err != nil {
		return nil, err
	}
	if len(tags) == 0 {
		return nil, transport.NewError(transport.KindSource, "no tags found in %s/%s", s.cfg.Owner, s.cfg.Repo)
	}
	sort.Slice(tags, func(i, j int) bool {
		return version.Compare(tags[i].Name, tags[j].Name) > 0
	})
	tag := tags[0].Name
	return &Release{Version: version.CleanTag(tag), TagName: tag}, nil
}

func (s *githubSource) fetchTags(ctx context.Context, perPage int) ([]githubTag, error) {
	var tags []githubTag
	if err := s.client.GetJSON(ctx, s.tagsURL(perPage), &tags); err != nil {
		return nil, err
	}
	return tags, nil
}

// List returns releases or tags, newest first.
func (s *githubSource) List(ctx context.Context, limit int) ([]*Release, error) {
	if limit <= 0 {
		limit = 10
	}
	if s.cfg.UseReleases {
		var rels []githubRelease
		if err := s.client.GetJSON(ctx, s.listURL(limit), &rels); err != nil {
			return nil, err
		}
		out := make([]*Release, 0, len(rels))
		for i := range rels {
			if rels[i].Draft {
				continue
			}
			r := s.fromGithubRelease(&rels[i])
			out = append(out, &r)
		}
		return out, nil
	}
	tags, err := s.fetchTags(ctx, limit)
	if err != nil {
		return nil, err
	}
	out := make([]*Release, 0, len(tags))
	for _, t := range tags {
		tag := t.Name
		out = append(out, &Release{Version: version.CleanTag(tag), TagName: tag})
	}
	return out, nil
}

func (s *githubSource) fromGithubRelease(r *githubRelease) Release {
	rel := Release{
		Version:     version.CleanTag(r.TagName),
		TagName:     r.TagName,
		PublishedAt: r.PublishedAt,
		Name:        r.Name,
		Notes:       r.Body,
	}
	for _, a := range r.Assets {
		rel.Assets = append(rel.Assets, Asset{Name: a.Name, URL: a.BrowserDownloadURL, Size: a.Size})
	}
	if len(rel.Assets) == 0 && r.TarballURL != "" {
		rel.Assets = append(rel.Assets, Asset{
			Name: fmt.Sprintf("%s-%s.tar.gz", s.cfg.Repo, r.TagName),
			URL:  r.TarballURL,
		})
	}
	return rel
}
