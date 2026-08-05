package lib

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writeConfig(t *testing.T, dir, content string) string {
	t.Helper()
	path := filepath.Join(dir, "config.json")
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestLibCheck(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"tag_name": "v2.0.0", "assets": []}`))
	}))
	defer srv.Close()

	dir := t.TempDir()
	cfg := writeConfig(t, dir, `{
		"product": {"name": "my-app", "current_version": "1.0.0"},
		"source": {"type": "github-tag", "github": {"owner": "acme", "repo": "my-app", "api_base_url": "`+srv.URL+`", "use_releases": true}}
	}`)

	out, err := Check(cfg, "", "", "", "")
	if err != nil {
		t.Fatalf("Check: %v", err)
	}
	if !strings.Contains(out, `"update_available":true`) || !strings.Contains(out, `"latest_version":"2.0.0"`) {
		t.Errorf("out = %s", out)
	}
}

func TestLibCheckError(t *testing.T) {
	out, err := Check("/nonexistent.json", "", "", "", "")
	if err == nil {
		t.Fatal("expected error")
	}
	if out != "" {
		t.Errorf("out should be empty on error, got %q", out)
	}
	if !strings.Contains(err.Error(), "exit 2") {
		t.Errorf("err = %v", err)
	}
}

func TestLibVersion(t *testing.T) {
	if v := Version(); v == "" {
		t.Error("empty version")
	}
}
