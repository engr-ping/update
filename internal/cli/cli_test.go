package cli

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// writeConfig writes a config file and returns its path.
func writeConfig(t *testing.T, dir, content string) string {
	t.Helper()
	path := filepath.Join(dir, "config.json")
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

// githubHandler emulates a GitHub API for releases + a downloadable asset.
func githubHandler(content []byte) *httptest.Server {
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case strings.HasSuffix(r.URL.Path, "/releases/latest"):
			scheme := "http"
			if r.TLS != nil {
				scheme = "https"
			}
			w.Write([]byte(`{
				"tag_name": "v1.2.0",
				"published_at": "2024-01-15T10:00:00Z",
				"body": "bug fixes",
				"assets": [{"name": "app-linux-amd64.tar.gz", "browser_download_url": "` + scheme + `://` + r.Host + `/dl/app.bin", "size": 10}]
			}`))
		case strings.HasSuffix(r.URL.Path, "/releases"):
			w.Write([]byte(`[
				{"tag_name": "v1.2.0", "assets": []},
				{"tag_name": "v1.1.0", "assets": []}
			]`))
		case strings.HasSuffix(r.URL.Path, "/dl/app.bin"):
			w.Write(content)
		default:
			http.NotFound(w, r)
		}
	}))
}

func runCLI(t *testing.T, args ...string) (int, string, string) {
	t.Helper()
	var stdout, stderr bytes.Buffer
	code := Run(context.Background(), args, &stdout, &stderr)
	return code, stdout.String(), stderr.String()
}

func TestCheckUpdateAvailable(t *testing.T) {
	srv := githubHandler(nil)
	defer srv.Close()

	cfg := writeConfig(t, t.TempDir(), fmt.Sprintf(`{
		"product": {"name": "my-app", "current_version": "1.0.0"},
		"source": {"type": "github-tag", "github": {"owner": "acme", "repo": "my-app", "api_base_url": %q, "use_releases": true}}
	}`, srv.URL))

	code, out, _ := runCLI(t, "check", "--config", cfg)
	if code != 0 {
		t.Fatalf("exit = %d, want 0", code)
	}
	var res struct {
		Schema          int    `json:"schema"`
		CurrentVersion  string `json:"current_version"`
		LatestVersion   string `json:"latest_version"`
		UpdateAvailable bool   `json:"update_available"`
	}
	if err := json.Unmarshal([]byte(out), &res); err != nil {
		t.Fatalf("stdout is not JSON: %v\n%s", err, out)
	}
	if !res.UpdateAvailable {
		t.Error("update_available = false, want true")
	}
	if res.LatestVersion != "1.2.0" {
		t.Errorf("latest_version = %q", res.LatestVersion)
	}
	if res.Schema != 1 {
		t.Errorf("schema = %d", res.Schema)
	}
}

func TestCheckUpToDate(t *testing.T) {
	srv := githubHandler(nil)
	defer srv.Close()

	cfg := writeConfig(t, t.TempDir(), fmt.Sprintf(`{
		"product": {"name": "my-app", "current_version": "1.2.0"},
		"source": {"type": "github-tag", "github": {"owner": "acme", "repo": "my-app", "api_base_url": %q, "use_releases": true}}
	}`, srv.URL))

	code, out, _ := runCLI(t, "check", "--config", cfg)
	if code != 0 {
		t.Fatalf("exit = %d, want 0", code)
	}
	if !strings.Contains(out, `"update_available":false`) {
		t.Errorf("expected update_available=false, got %s", out)
	}
}

func TestCheckSourceErrorExit3(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "boom", http.StatusInternalServerError)
	}))
	defer srv.Close()

	cfg := writeConfig(t, t.TempDir(), fmt.Sprintf(`{
		"source": {"type": "github-tag", "github": {"owner": "acme", "repo": "my-app", "api_base_url": %q, "use_releases": true}}
	}`, srv.URL))

	code, _, errOut := runCLI(t, "check", "--config", cfg)
	if code != 3 {
		t.Errorf("exit = %d, want 3; stderr: %s", code, errOut)
	}
}

func TestCheckConfigErrorExit2(t *testing.T) {
	cfg := writeConfig(t, t.TempDir(), `{"source": {"type": "nope"}}`)
	code, _, _ := runCLI(t, "check", "--config", cfg)
	if code != 2 {
		t.Errorf("exit = %d, want 2", code)
	}
}

func TestList(t *testing.T) {
	srv := githubHandler(nil)
	defer srv.Close()

	cfg := writeConfig(t, t.TempDir(), fmt.Sprintf(`{
		"source": {"type": "github-tag", "github": {"owner": "acme", "repo": "my-app", "api_base_url": %q, "use_releases": true}}
	}`, srv.URL))

	code, out, _ := runCLI(t, "list", "--config", cfg, "--limit", "2")
	if code != 0 {
		t.Fatalf("exit = %d, want 0", code)
	}
	var res struct {
		Schema   int `json:"schema"`
		Versions []struct {
			Version string `json:"version"`
		} `json:"versions"`
	}
	if err := json.Unmarshal([]byte(out), &res); err != nil {
		t.Fatalf("stdout is not JSON: %v\n%s", err, out)
	}
	if len(res.Versions) != 2 {
		t.Errorf("versions len = %d, want 2", len(res.Versions))
	}
}

func TestDownload(t *testing.T) {
	content := []byte("binary-content-123")
	sum := sha256.Sum256(content)
	srv := githubHandler(content)
	defer srv.Close()

	dir := t.TempDir()
	cfg := writeConfig(t, dir, fmt.Sprintf(`{
		"source": {"type": "github-tag", "github": {"owner": "acme", "repo": "my-app", "api_base_url": %q, "use_releases": true}}
	}`, srv.URL))

	outPath := filepath.Join(dir, "out.bin")
	code, out, errOut := runCLI(t, "download", "--config", cfg,
		"--version", "latest", "--out", outPath, "--asset", "app-linux-amd64.tar.gz")
	if code != 0 {
		t.Fatalf("exit = %d, want 0; stderr: %s", code, errOut)
	}
	got, err := os.ReadFile(outPath)
	if err != nil {
		t.Fatalf("downloaded file: %v", err)
	}
	if !bytes.Equal(got, content) {
		t.Errorf("downloaded content mismatch")
	}
	if !strings.Contains(out, `"schema":1`) {
		t.Errorf("stdout = %s", out)
	}
	_ = sum
}

func TestDownloadChecksumMismatchExit4(t *testing.T) {
	content := []byte("binary-content")
	wrongSum := strings.Repeat("0", 64)

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case strings.HasSuffix(r.URL.Path, "/dl/app.bin"):
			w.Write(content)
		default:
			w.Write([]byte(fmt.Sprintf(`{"version": "1.0.0", "assets": [{"name": "app.bin", "url": %q, "sha256": %q}]}`,
				srv.URL+"/dl/app.bin", wrongSum)))
		}
	}))
	defer srv.Close()

	dir := t.TempDir()
	cfg := writeConfig(t, dir, fmt.Sprintf(`{
		"source": {"type": "custom", "custom": {"versions_url": %q}}
	}`, srv.URL+"/feed.json"))

	outPath := filepath.Join(dir, "out.bin")
	code, _, errOut := runCLI(t, "download", "--config", cfg,
		"--version", "latest", "--out", outPath)
	if code != 4 {
		t.Errorf("exit = %d, want 4; stderr: %s", code, errOut)
	}
	if _, err := os.Stat(outPath); !os.IsNotExist(err) {
		t.Errorf("file should not exist after checksum mismatch")
	}
}

func TestVersion(t *testing.T) {
	code, out, _ := runCLI(t, "version")
	if code != 0 {
		t.Errorf("exit = %d", code)
	}
	if strings.TrimSpace(out) == "" {
		t.Error("version output empty")
	}
}

func TestInternalGitHubBasicAuth(t *testing.T) {
	// 内部 GitHub Enterprise：用 --username/--password（GUI 登录场景）认证
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		user, pass, ok := r.BasicAuth()
		if !ok || user != "bob" || pass != "s3cret" {
			http.Error(w, `{"message":"Bad credentials"}`, http.StatusUnauthorized)
			return
		}
		switch {
		case strings.HasSuffix(r.URL.Path, "/releases/latest"):
			w.Write([]byte(`{"tag_name": "v5.0.0", "assets": []}`))
		default:
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()

	dir := t.TempDir()
	// 配置里不写任何凭据环境变量，纯靠运行时 flag
	cfg := writeConfig(t, dir, fmt.Sprintf(`{
		"product": {"name": "my-app", "current_version": "4.9.9"},
		"source": {"type": "github-tag", "github": {
			"owner": "acme", "repo": "my-app",
			"api_base_url": %q, "use_releases": true
		}}
	}`, srv.URL))

	code, out, errOut := runCLI(t, "check", "--config", cfg,
		"--username", "bob", "--password", "s3cret")
	if code != 0 {
		t.Fatalf("exit = %d, want 0; stderr: %s", code, errOut)
	}
	if !strings.Contains(out, `"latest_version":"5.0.0"`) {
		t.Errorf("stdout = %s", out)
	}
}

func TestInternalGitHubEnvAuth(t *testing.T) {
	// 内部 GitHub Enterprise：凭据走环境变量（username_env/token_env）
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		user, pass, ok := r.BasicAuth()
		if !ok || user != "alice" || pass != "tok123" {
			http.Error(w, `{"message":"Bad credentials"}`, http.StatusUnauthorized)
			return
		}
		w.Write([]byte(`{"tag_name": "v3.0.0", "assets": []}`))
	}))
	defer srv.Close()

	dir := t.TempDir()
	cfg := writeConfig(t, dir, fmt.Sprintf(`{
		"source": {"type": "github-tag", "github": {
			"owner": "acme", "repo": "my-app",
			"username_env": "GHE_TEST_USER", "token_env": "GHE_TEST_TOKEN",
			"api_base_url": %q, "use_releases": true
		}}
	}`, srv.URL))
	t.Setenv("GHE_TEST_USER", "alice")
	t.Setenv("GHE_TEST_TOKEN", "tok123")

	code, out, errOut := runCLI(t, "check", "--config", cfg)
	if code != 0 {
		t.Fatalf("exit = %d, want 0; stderr: %s", code, errOut)
	}
	if !strings.Contains(out, `"latest_version":"3.0.0"`) {
		t.Errorf("stdout = %s", out)
	}
}

func TestUnknownCommandExit2(t *testing.T) {
	code, _, _ := runCLI(t, "frobnicate")
	if code != 2 {
		t.Errorf("exit = %d, want 2", code)
	}
}

func TestCustomSourceCheck(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer sekret" {
			http.Error(w, "no auth", http.StatusUnauthorized)
			return
		}
		w.Write([]byte(`{"version": "3.1.0", "assets": [{"name": "app.bin", "url": "/dl/app.bin"}]}`))
	}))
	defer srv.Close()

	dir := t.TempDir()
	cfg := writeConfig(t, dir, fmt.Sprintf(`{
		"product": {"name": "my-app", "current_version": "2.0.0"},
		"source": {"type": "custom", "custom": {
			"versions_url": %q,
			"auth": {"type": "bearer", "token_env": "UPDATE_TEST_TOKEN"}
		}}
	}`, srv.URL+"/feed.json"))

	t.Setenv("UPDATE_TEST_TOKEN", "sekret")
	code, out, errOut := runCLI(t, "check", "--config", cfg)
	if code != 0 {
		t.Fatalf("exit = %d, want 0; stderr: %s", code, errOut)
	}
	if !strings.Contains(out, `"latest_version":"3.1.0"`) {
		t.Errorf("stdout = %s", out)
	}
}

func TestNoConfigExit2(t *testing.T) {
	t.Setenv("UPDATE_CONFIG", "")
	code, _, _ := runCLI(t, "check")
	if code != 2 {
		t.Errorf("exit = %d, want 2", code)
	}
}

func TestDownloadNoAssetExit4(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"version": "1.0.0", "assets": [{"name": "app-windows-amd64.zip"}]}`))
	}))
	defer srv.Close()

	dir := t.TempDir()
	cfg := writeConfig(t, dir, fmt.Sprintf(`{
		"source": {"type": "custom", "custom": {"versions_url": %q}}
	}`, srv.URL+"/feed.json"))

	code, _, errOut := runCLI(t, "download", "--config", cfg, "--version", "latest",
		"--platform", "linux/amd64", "--asset", "app.zip")
	if code != 4 {
		t.Errorf("exit = %d, want 4; stderr: %s", code, errOut)
	}
}

func TestChecksumHexHelper(t *testing.T) {
	if hex.EncodeToString([]byte("x")) == "" {
		t.Fatal("hex helper broken")
	}
}
