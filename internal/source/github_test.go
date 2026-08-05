package source

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"update/internal/config"
	"update/internal/transport"
)

// clientFromConfig mirrors the wiring done by the CLI layer.
func clientFromConfig(cfg *config.Config) *transport.Client {
	var auth *transport.Auth
	var headers map[string]string
	switch cfg.Source.Type {
	case "github-tag":
		if g := cfg.Source.GitHub; g.Token != "" || g.Username != "" {
			auth = &transport.Auth{Type: "bearer", Token: g.Token}
			if g.Username != "" {
				auth = &transport.Auth{Type: "basic", Username: g.Username, Token: g.Token}
			}
		}
	case "custom":
		headers = cfg.Source.Custom.Headers
		if a := cfg.Source.Custom.Auth; a != nil {
			auth = &transport.Auth{Type: a.Type, Token: a.Token, Username: a.Username}
		}
	}
	return transport.New(transport.Options{Auth: auth, Headers: headers})
}

func newGitHubConfig(srvURL, token string) *config.Config {
	return &config.Config{
		Source: config.SourceConfig{
			Type: "github-tag",
			GitHub: &config.GitHubConfig{
				Owner:       "acme",
				Repo:        "my-app",
				APIBaseURL:  srvURL,
				UseReleases: true,
				Token:       token,
			},
		},
	}
}

func githubReleasesHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case strings.HasSuffix(r.URL.Path, "/releases/latest"):
			if r.Header.Get("Authorization") != "Bearer ghp_test" {
				http.Error(w, `{"message":"Bad credentials"}`, http.StatusUnauthorized)
				return
			}
			w.Write([]byte(`{
				"tag_name": "v1.2.0",
				"name": "v1.2.0",
				"published_at": "2024-01-15T10:00:00Z",
				"body": "bug fixes",
				"assets": [
					{"name": "app-linux-amd64.tar.gz", "browser_download_url": "/dl/app-linux-amd64.tar.gz", "size": 10},
					{"name": "app-windows-amd64.zip", "browser_download_url": "/dl/app-windows-amd64.zip", "size": 20}
				]
			}`))
		case strings.HasSuffix(r.URL.Path, "/releases"):
			w.Write([]byte(`[
				{"tag_name": "v1.2.0", "published_at": "2024-01-15T10:00:00Z", "assets": []},
				{"tag_name": "v1.1.0", "published_at": "2024-01-01T10:00:00Z", "assets": []}
			]`))
		default:
			http.NotFound(w, r)
		}
	})
}

func TestGitHubLatest(t *testing.T) {
	srv := httptest.NewServer(githubReleasesHandler())
	defer srv.Close()

	cfg := newGitHubConfig(srv.URL, "ghp_test")
	src, err := New(cfg, clientFromConfig(cfg))
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	rel, err := src.Latest(context.Background())
	if err != nil {
		t.Fatalf("Latest: %v", err)
	}
	if rel.Version != "1.2.0" {
		t.Errorf("Version = %q, want 1.2.0", rel.Version)
	}
	if len(rel.Assets) != 2 {
		t.Fatalf("Assets = %d, want 2", len(rel.Assets))
	}
	if rel.Assets[0].Name != "app-linux-amd64.tar.gz" {
		t.Errorf("Asset[0].Name = %q", rel.Assets[0].Name)
	}
}

func TestGitHubList(t *testing.T) {
	srv := httptest.NewServer(githubReleasesHandler())
	defer srv.Close()

	src, err := New(newGitHubConfig(srv.URL, "ghp_test"), clientFromConfig(newGitHubConfig(srv.URL, "ghp_test")))
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	rels, err := src.List(context.Background(), 10)
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(rels) != 2 {
		t.Fatalf("List len = %d, want 2", len(rels))
	}
	if rels[0].Version != "1.2.0" {
		t.Errorf("rels[0].Version = %q", rels[0].Version)
	}
}

func TestGitHubUnauthorized(t *testing.T) {
	srv := httptest.NewServer(githubReleasesHandler())
	defer srv.Close()

	src, _ := New(newGitHubConfig(srv.URL, "wrong_token"), clientFromConfig(newGitHubConfig(srv.URL, "wrong_token")))
	_, err := src.Latest(context.Background())
	if err == nil {
		t.Fatal("expected unauthorized error")
	}
	te, ok := err.(*transport.Error)
	if !ok || te.StatusCode != http.StatusUnauthorized {
		t.Errorf("error = %v, want 401", err)
	}
}

func TestGitHubTagsOnly(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/tags") {
			w.Write([]byte(`[
				{"name": "v1.0.0"},
				{"name": "v1.2.0"},
				{"name": "v1.1.0"}
			]`))
			return
		}
		http.NotFound(w, r)
	}))
	defer srv.Close()

	cfg := &config.Config{
		Source: config.SourceConfig{
			Type:   "github-tag",
			GitHub: &config.GitHubConfig{Owner: "acme", Repo: "my-app", APIBaseURL: srv.URL, UseReleases: false},
		},
	}
	src, _ := New(cfg, clientFromConfig(cfg))
	rel, err := src.Latest(context.Background())
	if err != nil {
		t.Fatalf("Latest: %v", err)
	}
	if rel.Version != "1.2.0" {
		t.Errorf("Version = %q, want 1.2.0", rel.Version)
	}
}

func TestGitHubFallbackToTagsOn404(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/tags") {
			w.Write([]byte(`[{"name": "v2.0.0"}]`))
			return
		}
		http.NotFound(w, r)
	}))
	defer srv.Close()

	src, _ := New(newGitHubConfig(srv.URL, ""), clientFromConfig(newGitHubConfig(srv.URL, "")))
	rel, err := src.Latest(context.Background())
	if err != nil {
		t.Fatalf("Latest: %v", err)
	}
	if rel.Version != "2.0.0" {
		t.Errorf("Version = %q, want 2.0.0", rel.Version)
	}
}
