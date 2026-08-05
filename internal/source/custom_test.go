package source

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"update/internal/config"
)

func newCustomConfig(srvURL string) *config.Config {
	return &config.Config{
		Source: config.SourceConfig{
			Type: "custom",
			Custom: &config.CustomConfig{
				VersionsURL:         srvURL + "/feed.json",
				DownloadURLTemplate: srvURL + "/files/{version}/{asset}",
				Auth:                &config.AuthConfig{Type: "bearer", Token: "tok"},
			},
		},
	}
}

func TestCustomSingleFeed(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer tok" {
			http.Error(w, "bad auth", http.StatusUnauthorized)
			return
		}
		w.Write([]byte(`{
			"version": "2.1.0",
			"published_at": "2024-03-01T00:00:00Z",
			"assets": [{"name": "app.zip"}]
		}`))
	}))
	defer srv.Close()

	src, err := New(newCustomConfig(srv.URL), clientFromConfig(newCustomConfig(srv.URL)))
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	rel, err := src.Latest(context.Background())
	if err != nil {
		t.Fatalf("Latest: %v", err)
	}
	if rel.Version != "2.1.0" {
		t.Errorf("Version = %q", rel.Version)
	}
	// template should fill the asset URL
	if got := rel.Assets[0].URL; got != srv.URL+"/files/2.1.0/app.zip" {
		t.Errorf("asset URL = %q", got)
	}
}

func TestCustomArrayFeedLatest(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`[
			{"version": "1.0.0"},
			{"version": "2.0.0"},
			{"version": "1.5.0"}
		]`))
	}))
	defer srv.Close()

	src, _ := New(newCustomConfig(srv.URL), clientFromConfig(newCustomConfig(srv.URL)))
	rel, err := src.Latest(context.Background())
	if err != nil {
		t.Fatalf("Latest: %v", err)
	}
	if rel.Version != "2.0.0" {
		t.Errorf("Version = %q, want 2.0.0", rel.Version)
	}

	rels, err := src.List(context.Background(), 2)
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(rels) != 2 || rels[0].Version != "1.0.0" || rels[1].Version != "2.0.0" {
		t.Errorf("List = %v, want [1.0.0 2.0.0]", rels)
	}
}

func TestCustomEmptyFeed(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`[]`))
	}))
	defer srv.Close()

	src, _ := New(newCustomConfig(srv.URL), clientFromConfig(newCustomConfig(srv.URL)))
	if _, err := src.Latest(context.Background()); err == nil {
		t.Fatal("expected error for empty feed")
	}
}

func TestCustomInvalidFeed(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`not json`))
	}))
	defer srv.Close()

	src, _ := New(newCustomConfig(srv.URL), clientFromConfig(newCustomConfig(srv.URL)))
	if _, err := src.Latest(context.Background()); err == nil {
		t.Fatal("expected error for invalid feed")
	}
}
