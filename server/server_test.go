package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// setup creates a data dir with a sample product and returns a test server.
func setup(t *testing.T) (*httptest.Server, string) {
	t.Helper()
	dir := t.TempDir()
	// my-app v2.0.0: two assets + meta.json
	v2 := filepath.Join(dir, "package", "my-app", "v2.0.0")
	mustMkdir(t, v2)
	mustWrite(t, filepath.Join(v2, "app-linux-amd64.tar.gz"), "linux-content")
	mustWrite(t, filepath.Join(v2, "app-windows-amd64.zip"), "win-content")
	mustWrite(t, filepath.Join(v2, "meta.json"), `{
		"name": "My App",
		"notes": "second release",
		"published_at": "2024-02-01T00:00:00Z",
		"assets": {"app-linux-amd64.tar.gz": {"sha256": "deadbeef"}}
	}`)
	// my-app v1.0.0: no meta.json
	v1 := filepath.Join(dir, "package", "my-app", "v1.0.0")
	mustMkdir(t, v1)
	mustWrite(t, filepath.Join(v1, "app-linux-amd64.tar.gz"), "old-linux")
	// other-app v0.1.0
	oa := filepath.Join(dir, "package", "other-app", "v0.1.0")
	mustMkdir(t, oa)
	mustWrite(t, filepath.Join(oa, "app.bin"), "other")

	srv := &Server{dir: dir}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /feed/", srv.handleFeedPath)
	mux.HandleFunc("GET /feeds.json", srv.handleFeeds)
	mux.HandleFunc("GET /package/{name}/{version}/{file...}", srv.handleDownload)
	ts := httptest.NewServer(mux)
	t.Cleanup(ts.Close)
	return ts, dir
}

func mustMkdir(t *testing.T, p string) {
	t.Helper()
	if err := os.MkdirAll(p, 0o755); err != nil {
		t.Fatal(err)
	}
}

func mustWrite(t *testing.T, p, content string) {
	t.Helper()
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestFeedSortedNewestFirst(t *testing.T) {
	ts, _ := setup(t)
	resp, err := http.Get(ts.URL + "/feed/my-app.json")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d", resp.StatusCode)
	}
	var releases []Release
	if err := json.NewDecoder(resp.Body).Decode(&releases); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if len(releases) != 2 {
		t.Fatalf("releases = %d, want 2", len(releases))
	}
	if releases[0].Version != "v2.0.0" || releases[1].Version != "v1.0.0" {
		t.Errorf("order = [%s %s], want [v2.0.0 v1.0.0]",
			releases[0].Version, releases[1].Version)
	}
	if releases[0].Notes != "second release" {
		t.Errorf("notes = %q", releases[0].Notes)
	}
	if releases[0].PublishedAt != "2024-02-01T00:00:00Z" {
		t.Errorf("published_at = %q", releases[0].PublishedAt)
	}
	// assets: meta.json excluded, sha256 merged
	if len(releases[0].Assets) != 2 {
		t.Fatalf("assets = %d, want 2", len(releases[0].Assets))
	}
	if releases[0].Assets[0].SHA256 != "deadbeef" {
		t.Errorf("sha256 = %q", releases[0].Assets[0].SHA256)
	}
	// URL must point to the download endpoint
	if !strings.Contains(releases[0].Assets[0].URL, "/package/my-app/v2.0.0/") {
		t.Errorf("asset url = %q", releases[0].Assets[0].URL)
	}
	// v1.0.0 without meta: published_at falls back to dir mtime (non-empty)
	if releases[1].PublishedAt == "" {
		t.Error("v1.0.0 published_at empty, want fallback")
	}
}

func TestFeedSemverOrdering(t *testing.T) {
	dir := t.TempDir()
	root := filepath.Join(dir, "package", "app")
	for _, v := range []string{"v1.9.0", "v1.10.0", "v2.0.0", "v1.2.0"} {
		mustMkdir(t, filepath.Join(root, v))
	}
	srv := &Server{dir: dir}
	ts := httptest.NewServer(http.HandlerFunc(srv.handleFeedPath))
	defer ts.Close()

	resp, err := http.Get(ts.URL + "/feed/app.json")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	var releases []Release
	_ = json.NewDecoder(resp.Body).Decode(&releases)
	if len(releases) != 4 {
		t.Fatalf("releases = %d", len(releases))
	}
	got := []string{releases[0].Version, releases[1].Version, releases[2].Version, releases[3].Version}
	want := []string{"v2.0.0", "v1.10.0", "v1.9.0", "v1.2.0"}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("order = %v, want %v", got, want)
		}
	}
}

func TestFeedUnknownProduct404(t *testing.T) {
	ts, _ := setup(t)
	resp, err := http.Get(ts.URL + "/feed/nope.json")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusNotFound {
		t.Errorf("status = %d, want 404", resp.StatusCode)
	}
}

func TestFeedsList(t *testing.T) {
	ts, _ := setup(t)
	resp, err := http.Get(ts.URL + "/feeds.json")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	var out struct {
		Feeds []map[string]interface{} `json:"feeds"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if len(out.Feeds) != 2 {
		t.Fatalf("feeds = %d, want 2", len(out.Feeds))
	}
	if out.Feeds[0]["name"] != "my-app" || out.Feeds[0]["latest_version"] != "v2.0.0" {
		t.Errorf("feed[0] = %v", out.Feeds[0])
	}
}

func TestDownload(t *testing.T) {
	ts, _ := setup(t)
	resp, err := http.Get(ts.URL + "/package/my-app/v2.0.0/app-linux-amd64.tar.gz")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d", resp.StatusCode)
	}
	buf := make([]byte, 64)
	n, _ := resp.Body.Read(buf)
	if string(buf[:n]) != "linux-content" {
		t.Errorf("content = %q", buf[:n])
	}
}

func TestDownload404(t *testing.T) {
	ts, _ := setup(t)
	for _, path := range []string{
		"/package/my-app/v2.0.0/nope.bin",
		"/package/my-app/v9.9.9/app.bin",
		"/package/nope/v1.0.0/app.bin",
	} {
		resp, err := http.Get(ts.URL + path)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		if resp.StatusCode != http.StatusNotFound {
			t.Errorf("%s: status = %d, want 404", path, resp.StatusCode)
		}
	}
}

func TestPathTraversalRejected(t *testing.T) {
	ts, dir := setup(t)
	secret := filepath.Join(dir, "secret.txt")
	mustWrite(t, secret, "top-secret")

	for _, path := range []string{
		"/package/../secret.txt",
		"/package/my-app/..%2F..%2Fsecret.txt",
		"/package/my-app/v2.0.0/../../secret.txt",
	} {
		resp, err := http.Get(ts.URL + path)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		if resp.StatusCode == http.StatusOK {
			t.Errorf("traversal %q returned 200, want rejected", path)
		}
	}
}

func TestInvalidProductName(t *testing.T) {
	srv := &Server{dir: t.TempDir()}
	ts := httptest.NewServer(http.HandlerFunc(srv.handleFeedPath))
	defer ts.Close()
	resp, err := http.Get(ts.URL + "/feed/..%2F..%2Fetc.json")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusBadRequest {
		t.Errorf("status = %d, want 400", resp.StatusCode)
	}
}

func TestProductDirRejectsTraversal(t *testing.T) {
	srv := &Server{dir: t.TempDir()}
	for _, name := range []string{"..", "../x", "a/b/../../x", "/abs", ""} {
		if _, err := srv.productDir(name); err == nil {
			t.Errorf("productDir(%q) accepted", name)
		}
	}
}

func TestAssetURLScheme(t *testing.T) {
	r := httptest.NewRequest("GET", "http://host/package/a/b/f", nil)
	u := assetURL(r, "a", "b", "f")
	if u != "http://host/package/a/b/f" {
		t.Errorf("url = %q", u)
	}
	r.Header.Set("X-Forwarded-Proto", "https")
	if u := assetURL(r, "a", "b", "f"); u != "https://host/package/a/b/f" {
		t.Errorf("https url = %q", u)
	}
}
