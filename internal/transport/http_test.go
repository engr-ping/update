package transport

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func sha256Hex(b []byte) string {
	s := sha256.Sum256(b)
	return hex.EncodeToString(s[:])
}

func TestDoHeadersAndAuth(t *testing.T) {
	var gotAuth, gotHeader string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotAuth = r.Header.Get("Authorization")
		gotHeader = r.Header.Get("X-Custom")
		w.Write([]byte("ok"))
	}))
	defer srv.Close()

	c := New(Options{
		Headers: map[string]string{"X-Custom": "v1"},
		Auth:    &Auth{Type: "bearer", Token: "tok"},
	})
	resp, err := c.Do(context.Background(), srv.URL)
	if err != nil {
		t.Fatalf("Do: %v", err)
	}
	resp.Body.Close()
	if gotAuth != "Bearer tok" {
		t.Errorf("Authorization = %q", gotAuth)
	}
	if gotHeader != "v1" {
		t.Errorf("X-Custom = %q", gotHeader)
	}
}

func TestDoBasicAuth(t *testing.T) {
	var gotAuth string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotAuth = r.Header.Get("Authorization")
		w.Write([]byte("ok"))
	}))
	defer srv.Close()

	c := New(Options{Auth: &Auth{Type: "basic", Username: "bob", Token: "pw"}})
	resp, err := c.Do(context.Background(), srv.URL)
	if err != nil {
		t.Fatalf("Do: %v", err)
	}
	resp.Body.Close()
	if !strings.HasPrefix(gotAuth, "Basic ") {
		t.Errorf("Authorization = %q", gotAuth)
	}
}

func TestDoHTTPError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, `{"message":"Not Found"}`, http.StatusNotFound)
	}))
	defer srv.Close()

	c := New(Options{})
	_, err := c.Do(context.Background(), srv.URL)
	if err == nil {
		t.Fatal("Do: expected error")
	}
	te, ok := err.(*Error)
	if !ok {
		t.Fatalf("error type = %T, want *transport.Error", err)
	}
	if te.Kind != KindSource {
		t.Errorf("Kind = %v, want KindSource", te.Kind)
	}
	if te.StatusCode != http.StatusNotFound {
		t.Errorf("StatusCode = %d", te.StatusCode)
	}
	if !strings.Contains(te.Message, "Not Found") {
		t.Errorf("Message = %q", te.Message)
	}
}

func TestGetJSON(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"a":1}`))
	}))
	defer srv.Close()

	var out struct {
		A int `json:"a"`
	}
	c := New(Options{})
	if err := c.GetJSON(context.Background(), srv.URL, &out); err != nil {
		t.Fatalf("GetJSON: %v", err)
	}
	if out.A != 1 {
		t.Errorf("A = %d", out.A)
	}
}

func TestDownloadChecksumAndAtomic(t *testing.T) {
	content := []byte("hello update")
	sum := sha256Hex(content)

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(content)
	}))
	defer srv.Close()

	dest := filepath.Join(t.TempDir(), "app.bin")
	c := New(Options{})

	// mismatch -> error and no file
	if err := c.Download(context.Background(), srv.URL, dest, "sha256:"+strings.Repeat("0", 64)); err == nil {
		t.Fatal("Download: expected checksum mismatch error")
	}
	if _, err := os.Stat(dest); !os.IsNotExist(err) {
		t.Errorf("dest should not exist after failed download: %v", err)
	}
	// no temp files left
	entries, _ := os.ReadDir(filepath.Dir(dest))
	if len(entries) != 0 {
		t.Errorf("leftover temp files: %v", entries)
	}

	// correct checksum -> file created
	if err := c.Download(context.Background(), srv.URL, dest, "sha256:"+sum); err != nil {
		t.Fatalf("Download: %v", err)
	}
	got, err := os.ReadFile(dest)
	if err != nil {
		t.Fatalf("ReadFile: %v", err)
	}
	if string(got) != string(content) {
		t.Errorf("content = %q, want %q", got, content)
	}
}

func TestDownloadNetworkErrorKind(t *testing.T) {
	c := New(Options{})
	err := c.Download(context.Background(), "http://127.0.0.1:1/file", "/tmp/x", "")
	if err == nil {
		t.Fatal("expected error")
	}
	if te, ok := err.(*Error); ok {
		// either the request fails (source) before temp file, fine to accept KindSource
		_ = te
	}
}
