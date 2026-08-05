package version

import "testing"

func TestMatchAsset(t *testing.T) {
	tests := []struct {
		name, os, arch string
		want           bool
	}{
		{"app-linux-amd64.tar.gz", "linux", "amd64", true},
		{"app-linux-arm64.tar.gz", "linux", "arm64", true},
		{"app-windows-amd64.zip", "windows", "amd64", true},
		{"app-linux_amd64.deb", "linux", "amd64", true},
		{"app.linux.amd64.rpm", "linux", "amd64", true},
		{"app-amd64-linux.tar.gz", "linux", "amd64", true},
		{"app-darwin-x86_64.zip", "darwin", "amd64", true},
		{"app-linux-amd64.tar.gz", "windows", "amd64", false},
		{"app-windows-amd64.zip", "linux", "amd64", false},
		{"app-x86_64.exe", "windows", "amd64", false}, // no os marker, not neutral (has arch marker + .exe missing windows) -> but hasPlatformMarker true so not neutral
		{"README.md", "linux", "amd64", true},         // neutral
		{"LICENSE", "windows", "arm64", true},         // neutral
		{"checksums.txt", "linux", "amd64", true},     // neutral
		{"", "linux", "amd64", true},                  // empty treated as neutral
		{"app-linux-amd64", "", "", true},             // no platform given -> all
	}
	for _, tt := range tests {
		got := MatchAsset(tt.name, tt.os, tt.arch)
		if got != tt.want {
			t.Errorf("MatchAsset(%q, %q, %q) = %v, want %v", tt.name, tt.os, tt.arch, got, tt.want)
		}
	}
}
