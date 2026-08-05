// Package versioninfo holds build-time version metadata injected via ldflags.
package versioninfo

var (
	// Version is the semantic version of the update binary itself.
	Version = "dev"
	// Commit is the git commit it was built from.
	Commit = "none"
	// Date is the UTC build timestamp.
	Date = "unknown"
)

// String returns the version string.
func String() string { return Version }
