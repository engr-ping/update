// Package transport provides the HTTP client used by all release sources.
// It handles authentication, custom headers, TLS, timeouts and normalizes
// HTTP/network failures into typed Errors that the CLI maps to exit codes.
package transport

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// Kind classifies a failure for CLI exit-code mapping.
type Kind int

const (
	// KindSource: network, HTTP status or upstream parse failure.
	KindSource Kind = iota
	// KindDownload: file download/checksum/write failure.
	KindDownload
)

// Error is a typed transport failure.
type Error struct {
	Kind       Kind
	Message    string
	StatusCode int
}

func (e *Error) Error() string { return e.Message }

// NewError builds a transport error.
func NewError(kind Kind, format string, args ...interface{}) *Error {
	return &Error{Kind: kind, Message: fmt.Sprintf(format, args...)}
}

// Auth carries resolved credentials for a request.
type Auth struct {
	Type     string // "", "bearer" or "basic"
	Token    string
	Username string
}

// Options configures a Client.
type Options struct {
	Timeout time.Duration
	Headers map[string]string
	Auth    *Auth
	// Insecure skips TLS verification. Intended only for custom sources
	// on private networks that use self-signed certs.
	Insecure bool
}

// Client is a shared HTTP client. It is safe for concurrent use.
type Client struct {
	http   *http.Client
	header http.Header
	auth   *Auth
}

// New builds a Client with the given options. Timeout defaults to 30s.
func New(opts Options) *Client {
	if opts.Timeout <= 0 {
		opts.Timeout = 30 * time.Second
	}
	tr := http.DefaultTransport.(*http.Transport).Clone()
	if opts.Insecure {
		// #nosec G402 -- explicit opt-in for private networks with self-signed certs.
		tr.TLSClientConfig = &tls.Config{InsecureSkipVerify: true}
	}
	c := &Client{
		http:   &http.Client{Timeout: opts.Timeout, Transport: tr},
		header: make(http.Header),
		auth:   opts.Auth,
	}
	for k, v := range opts.Headers {
		c.header.Set(k, v)
	}
	return c
}

// Do performs a GET request, applying headers and auth, and returns an open
// response body for status 2xx. The caller must close resp.Body.
func (c *Client) Do(ctx context.Context, rawURL string) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return nil, NewError(KindSource, "invalid url %q: %v", rawURL, err)
	}
	for k, vs := range c.header {
		for _, v := range vs {
			req.Header.Add(k, v)
		}
	}
	if c.auth != nil {
		switch c.auth.Type {
		case "bearer":
			if c.auth.Token != "" {
				req.Header.Set("Authorization", "Bearer "+c.auth.Token)
			}
		case "basic":
			req.SetBasicAuth(c.auth.Username, c.auth.Token)
		}
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, NewError(KindSource, "request %s: %v", rawURL, err)
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		resp.Body.Close()
		return nil, &Error{
			Kind:       KindSource,
			StatusCode: resp.StatusCode,
			Message:    fmt.Sprintf("request %s: status %d: %s", rawURL, resp.StatusCode, strings.TrimSpace(string(body))),
		}
	}
	return resp, nil
}

// GetJSON fetches rawURL and decodes the body into out.
func (c *Client) GetJSON(ctx context.Context, rawURL string, out interface{}) error {
	resp, err := c.Do(ctx, rawURL)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	dec := json.NewDecoder(io.LimitReader(resp.Body, 32<<20))
	if err := dec.Decode(out); err != nil {
		return NewError(KindSource, "decode response from %s: %v", rawURL, err)
	}
	return nil
}

// Download streams rawURL to dest. The file is written to a temp file in the
// same directory and atomically renamed on success, so a partial download
// never leaves a broken file behind. If expectSHA256 is non-empty the
// download is verified and the temp file is removed on mismatch.
func (c *Client) Download(ctx context.Context, rawURL, dest, expectSHA256 string) error {
	resp, err := c.Do(ctx, rawURL)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	dir := filepath.Dir(dest)
	if dir == "" {
		dir = "."
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return NewError(KindDownload, "create dir %s: %v", dir, err)
	}
	tmp, err := os.CreateTemp(dir, "."+filepath.Base(dest)+".tmp-*")
	if err != nil {
		return NewError(KindDownload, "create temp file: %v", err)
	}
	defer os.Remove(tmp.Name()) // no-op after successful rename

	hash := sha256.New()
	if _, err := io.Copy(io.MultiWriter(tmp, hash), resp.Body); err != nil {
		tmp.Close()
		return NewError(KindDownload, "download %s: %v", rawURL, err)
	}
	if err := tmp.Close(); err != nil {
		return NewError(KindDownload, "close temp file: %v", err)
	}

	expect := strings.TrimPrefix(strings.ToLower(expectSHA256), "sha256:")
	if expect != "" && expect != hex.EncodeToString(hash.Sum(nil)) {
		return NewError(KindDownload, "checksum mismatch for %s: expected sha256:%s, got sha256:%s",
			rawURL, expect, hex.EncodeToString(hash.Sum(nil)))
	}
	if err := os.Rename(tmp.Name(), dest); err != nil {
		return NewError(KindDownload, "rename to %s: %v", dest, err)
	}
	return nil
}
