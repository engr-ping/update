package config

import (
	"os"
	"path/filepath"
	"testing"
)

func getenv(env map[string]string) func(string) string {
	return func(k string) string { return env[k] }
}

func TestParseGitHubReleases(t *testing.T) {
	data := []byte(`{
		"product": {"name": "my-app", "current_version": "1.0.0"},
		"source": {
			"type": "github-tag",
			"github": {"owner": "acme", "repo": "my-app", "token_env": "GITHUB_TOKEN", "use_releases": true}
		}
	}`)
	cfg, err := Parse(data, getenv(map[string]string{"GITHUB_TOKEN": "ghp_secret"}))
	if err != nil {
		t.Fatalf("Parse: %v", err)
	}
	if cfg.Source.GitHub.Owner != "acme" {
		t.Errorf("owner = %q", cfg.Source.GitHub.Owner)
	}
	if cfg.Source.GitHub.APIBaseURL != defaultGitHubAPIBaseURL {
		t.Errorf("api_base_url default = %q", cfg.Source.GitHub.APIBaseURL)
	}
	if cfg.Source.GitHub.Token != "ghp_secret" {
		t.Errorf("token not resolved from env")
	}
	if !cfg.Source.GitHub.UseReleases {
		t.Errorf("use_releases = false, want true")
	}
}

func TestParseCustomBearer(t *testing.T) {
	data := []byte(`{
		"source": {
			"type": "custom",
			"custom": {
				"versions_url": "https://updates.example.com/feed.json",
				"headers": {"X-Client": "my-app"},
				"auth": {"type": "bearer", "token_env": "UPDATE_TOKEN"}
			}
		}
	}`)
	cfg, err := Parse(data, getenv(map[string]string{"UPDATE_TOKEN": "tok123"}))
	if err != nil {
		t.Fatalf("Parse: %v", err)
	}
	if cfg.Source.Custom.Auth.Token != "tok123" {
		t.Errorf("bearer token not resolved")
	}
	if cfg.Source.Custom.Headers["X-Client"] != "my-app" {
		t.Errorf("custom headers lost")
	}
}

func TestParseBasic(t *testing.T) {
	data := []byte(`{
		"source": {
			"type": "custom",
			"custom": {
				"versions_url": "https://u.example.com/feed.json",
				"auth": {"type": "basic", "username_env": "U_USER", "token_env": "U_PASS"}
			}
		}
	}`)
	cfg, err := Parse(data, getenv(map[string]string{"U_USER": "bob", "U_PASS": "pw"}))
	if err != nil {
		t.Fatalf("Parse: %v", err)
	}
	a := cfg.Source.Custom.Auth
	if a.Username != "bob" || a.Token != "pw" {
		t.Errorf("basic credentials not resolved: %+v", a)
	}
}

func TestParseGitHubBasicAuth(t *testing.T) {
	data := []byte(`{
		"product": {"name": "my-app", "current_version": "1.0.0"},
		"source": {
			"type": "github-tag",
			"github": {
				"owner": "acme", "repo": "my-app",
				"username_env": "GHE_USER", "token_env": "GHE_TOKEN",
				"api_base_url": "https://github.internal.example.com/api/v3"
			}
		}
	}`)
	cfg, err := Parse(data, getenv(map[string]string{"GHE_USER": "bob", "GHE_TOKEN": "s3cret"}))
	if err != nil {
		t.Fatalf("Parse: %v", err)
	}
	g := cfg.Source.GitHub
	if g.Username != "bob" {
		t.Errorf("username = %q, want bob", g.Username)
	}
	if g.Token != "s3cret" {
		t.Errorf("token = %q, want s3cret", g.Token)
	}
	if g.APIBaseURL != "https://github.internal.example.com/api/v3" {
		t.Errorf("api_base_url = %q", g.APIBaseURL)
	}
}

func TestParseErrors(t *testing.T) {
	tests := []struct {
		name string
		data []byte
	}{
		{"missing type", []byte(`{"source": {}}`)},
		{"bad type", []byte(`{"source": {"type": "nope"}}`)},
		{"github missing section", []byte(`{"source": {"type": "github-tag"}}`)},
		{"github missing owner", []byte(`{"source": {"type": "github-tag", "github": {"repo": "r"}}}`)},
		{"custom missing section", []byte(`{"source": {"type": "custom"}}`)},
		{"custom missing url", []byte(`{"source": {"type": "custom", "custom": {}}}`)},
		{"bad auth type", []byte(`{"source": {"type": "custom", "custom": {"versions_url": "https://x/", "auth": {"type": "digest"}}}}`)},
		{"invalid json", []byte(`{not json`)},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := Parse(tt.data, os.Getenv); err == nil {
				t.Errorf("Parse(%s) = nil error, want error", tt.name)
			}
		})
	}
}

func TestLoadFromFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.json")
	if err := os.WriteFile(path, []byte(`{"source":{"type":"github-tag","github":{"owner":"o","repo":"r"}}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg, err := Load(path, nil)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if cfg.Source.GitHub.Owner != "o" {
		t.Errorf("owner = %q", cfg.Source.GitHub.Owner)
	}
}
