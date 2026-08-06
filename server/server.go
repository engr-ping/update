package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"update/internal/version"
)

// Server serves release feeds and artifacts from a data directory.
type Server struct {
	dir string
}

// metaFile is the optional per-version metadata file.
// Its presence is optional; everything except notes/checksum defaults
// are derived from the filesystem.
type metaFile struct {
	Name        string               `json:"name,omitempty"`
	Notes       string               `json:"notes,omitempty"`
	PublishedAt string               `json:"published_at,omitempty"`
	Checksum    string               `json:"checksum,omitempty"`
	Assets      map[string]assetMeta `json:"assets,omitempty"`
}

type assetMeta struct {
	SHA256 string `json:"sha256,omitempty"`
	Size   int64  `json:"size,omitempty"`
}

// Asset is a downloadable file in a release.
type Asset struct {
	Name   string `json:"name"`
	URL    string `json:"url"`
	Size   int64  `json:"size,omitempty"`
	SHA256 string `json:"sha256,omitempty"`
}

// Release mirrors the client-side unified release model (docs/design.md §6)
// so the feed can be consumed by `update check`/`list` unchanged.
type Release struct {
	Version     string  `json:"version"`
	PublishedAt string  `json:"published_at,omitempty"`
	Name        string  `json:"name,omitempty"`
	Notes       string  `json:"notes,omitempty"`
	Checksum    string  `json:"checksum,omitempty"`
	Assets      []Asset `json:"assets"`
}

// productDir returns <dir>/package/<name>.
func (s *Server) productDir(name string) (string, error) {
	if name == "" || strings.Contains(name, `\`) {
		return "", fmt.Errorf("invalid product name %q", name)
	}
	clean := filepath.Clean(name)
	if clean == "." || filepath.IsAbs(clean) {
		return "", fmt.Errorf("invalid product name %q", name)
	}
	for _, part := range strings.Split(name, "/") {
		if part == ".." {
			return "", fmt.Errorf("invalid product name %q", name)
		}
	}
	return filepath.Join(s.dir, "package", clean), nil
}

// listProducts returns product names (subdirectories of package/).
func (s *Server) listProducts() ([]string, error) {
	base := filepath.Join(s.dir, "package")
	entries, err := os.ReadDir(base)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var names []string
	for _, e := range entries {
		if e.IsDir() {
			names = append(names, e.Name())
		}
	}
	sort.Strings(names)
	return names, nil
}

// listVersions returns the version directories of a product, sorted by
// semver descending (newest first).
func (s *Server) listVersions(name string) ([]string, error) {
	pdir, err := s.productDir(name)
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(pdir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var vs []string
	for _, e := range entries {
		if e.IsDir() {
			vs = append(vs, e.Name())
		}
	}
	sort.Slice(vs, func(i, j int) bool {
		return version.Compare(vs[i], vs[j]) > 0
	})
	return vs, nil
}

// loadMeta reads <version>/meta.json if present (nil if absent).
func (s *Server) loadMeta(pdir, ver string) (*metaFile, error) {
	data, err := os.ReadFile(filepath.Join(pdir, ver, "meta.json"))
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var m metaFile
	if err := json.Unmarshal(data, &m); err != nil {
		return nil, fmt.Errorf("parse meta.json in %s/%s: %w", pdir, ver, err)
	}
	return &m, nil
}

// buildRelease assembles one release from the filesystem.
// assetURLs maps each file name to its download URL.
func (s *Server) buildRelease(pdir, ver string, assetURLs map[string]string) (*Release, error) {
	meta, err := s.loadMeta(pdir, ver)
	if err != nil {
		return nil, err
	}
	r := &Release{Version: ver}
	if meta != nil {
		r.Name = meta.Name
		r.Notes = meta.Notes
		r.PublishedAt = meta.PublishedAt
		r.Checksum = meta.Checksum
	}

	files, err := os.ReadDir(filepath.Join(pdir, ver))
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	for _, f := range files {
		if f.IsDir() || f.Name() == "meta.json" {
			continue
		}
		info, err := f.Info()
		if err != nil {
			return nil, err
		}
		a := Asset{
			Name: f.Name(),
			URL:  assetURLs[f.Name()],
			Size: info.Size(),
		}
		if meta != nil {
			if am, ok := meta.Assets[f.Name()]; ok {
				a.SHA256 = am.SHA256
				if am.Size > 0 {
					a.Size = am.Size
				}
			}
		}
		r.Assets = append(r.Assets, a)
	}
	if r.PublishedAt == "" && len(files) > 0 {
		if info, err := os.Stat(filepath.Join(pdir, ver)); err == nil {
			r.PublishedAt = info.ModTime().UTC().Format("2006-01-02T15:04:05Z")
		}
	}
	sort.Slice(r.Assets, func(i, j int) bool { return r.Assets[i].Name < r.Assets[j].Name })
	return r, nil
}

// assetURL builds the absolute download URL for one artifact from the
// incoming request.
func assetURL(r *http.Request, name, ver, file string) string {
	scheme := "http"
	if r.TLS != nil {
		scheme = "https"
	}
	if fwd := r.Header.Get("X-Forwarded-Proto"); fwd == "https" {
		scheme = "https"
	}
	return fmt.Sprintf("%s://%s/package/%s/%s/%s", scheme, r.Host, name, ver, file)
}

func writeJSON(w http.ResponseWriter, status int, v interface{}) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(v)
}

func writeError(w http.ResponseWriter, status int, format string, args ...interface{}) {
	writeJSON(w, status, map[string]string{"error": fmt.Sprintf(format, args...)})
}

// handleFeedPath serves GET /feed/<name>.json (manual parse: the mux
// subtree pattern /feed/ cannot use a {name}.json wildcard without
// conflicting with /feeds.json).
func (s *Server) handleFeedPath(w http.ResponseWriter, r *http.Request) {
	rest := strings.TrimPrefix(r.URL.Path, "/feed/")
	name := strings.TrimSuffix(rest, ".json")
	if name == rest || name == "" || strings.Contains(name, "/") {
		writeError(w, http.StatusBadRequest, "invalid feed path %q (want /feed/<name>.json)", r.URL.Path)
		return
	}
	s.handleFeedByName(w, r, name)
}

func (s *Server) handleFeedByName(w http.ResponseWriter, r *http.Request, name string) {
	pdir, err := s.productDir(name)
	if err != nil {
		writeError(w, http.StatusBadRequest, "%v", err)
		return
	}
	vers, err := s.listVersions(name)
	if err != nil {
		log.Printf("updateserver: list versions %s: %v", name, err)
		writeError(w, http.StatusInternalServerError, "list versions: %v", err)
		return
	}
	if len(vers) == 0 {
		writeError(w, http.StatusNotFound, "no versions for %q", name)
		return
	}
	releases := make([]*Release, 0, len(vers))
	for _, ver := range vers {
		urls := make(map[string]string)
		rdir := filepath.Join(pdir, ver)
		if files, err := os.ReadDir(rdir); err == nil {
			for _, f := range files {
				if !f.IsDir() && f.Name() != "meta.json" {
					urls[f.Name()] = assetURL(r, name, ver, f.Name())
				}
			}
		}
		rel, err := s.buildRelease(pdir, ver, urls)
		if err != nil {
			log.Printf("updateserver: build release %s/%s: %v", name, ver, err)
			writeError(w, http.StatusInternalServerError, "build release: %v", err)
			return
		}
		releases = append(releases, rel)
	}
	writeJSON(w, http.StatusOK, releases)
}

// handleFeeds serves GET /feeds.json — every product with its versions.
func (s *Server) handleFeeds(w http.ResponseWriter, r *http.Request) {
	names, err := s.listProducts()
	if err != nil {
		log.Printf("updateserver: list products: %v", err)
		writeError(w, http.StatusInternalServerError, "list products: %v", err)
		return
	}
	feeds := make([]map[string]interface{}, 0, len(names))
	for _, name := range names {
		vers, err := s.listVersions(name)
		if err != nil {
			log.Printf("updateserver: versions %s: %v", name, err)
			continue
		}
		if len(vers) == 0 {
			continue
		}
		feeds = append(feeds, map[string]interface{}{
			"name":           name,
			"latest_version": vers[0],
			"versions":       vers,
		})
	}
	writeJSON(w, http.StatusOK, map[string]interface{}{"feeds": feeds})
}

// handleDownload serves GET /package/<name>/<version>/<file>.
func (s *Server) handleDownload(w http.ResponseWriter, r *http.Request) {
	name, ver, file := r.PathValue("name"), r.PathValue("version"), r.PathValue("file")
	pdir, err := s.productDir(name)
	if err != nil {
		writeError(w, http.StatusBadRequest, "%v", err)
		return
	}
	full := filepath.Join(pdir, ver, file)
	// defense in depth: resolve and confirm it stays under the product dir
	resolved, err := filepath.EvalSymlinks(full)
	if err != nil || !strings.HasPrefix(resolved, filepath.Clean(pdir)+string(filepath.Separator)) {
		writeError(w, http.StatusNotFound, "not found")
		return
	}
	http.ServeFile(w, r, resolved)
}
