// Command updateserver is the server side of the update project: a
// read-only HTTP distribution server. Artifacts live under
//
//	<dir>/package/<name>/<version>/<file>
//
// It serves three kinds of endpoints:
//
//	GET /feed/<name>.json       unified release feed for one product
//	GET /feeds.json             list of all products
//	GET /package/<name>/<version>/<file>   artifact download
//
// The feed format is exactly the "custom source" protocol consumed by the
// update CLI (docs/design.md §6), so a client can point versions_url at
// /feed/<name>.json directly.
package main

import (
	"flag"
	"log"
	"net/http"
	"os"
)

func main() {
	addr := flag.String("addr", ":8080", "listen address")
	dir := flag.String("dir", "./package", "data directory containing package/<name>/<version>/")
	flag.Parse()

	if err := os.MkdirAll(*dir, 0o755); err != nil {
		log.Fatalf("updateserver: create data dir: %v", err)
	}

	srv := &Server{dir: *dir}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /feed/", srv.handleFeedPath)
	mux.HandleFunc("GET /feeds.json", srv.handleFeeds)
	mux.HandleFunc("GET /package/{name}/{version}/{file...}", srv.handleDownload)
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"status":"ok"}`))
	})

	log.Printf("updateserver listening on %s (dir=%s)", *addr, *dir)
	if err := http.ListenAndServe(*addr, mux); err != nil {
		log.Fatalf("updateserver: %v", err)
	}
}
