package version

import "strings"

// CleanTag strips common prefixes (v, V, release-) from a version tag,
// returning the bare version used for comparison and display.
func CleanTag(tag string) string {
	t := strings.TrimPrefix(tag, "v")
	t = strings.TrimPrefix(t, "V")
	t = strings.TrimPrefix(t, "release-")
	return t
}

// MatchAsset reports whether an asset should be selected for the given
// platform. When osName or arch is empty, or equals "all", every asset
// matches. Otherwise it matches on common os-arch naming conventions
// ("linux-amd64", "linux_amd64", "windows-x86_64", ...). Assets with no
// recognizable platform marker are treated as platform-neutral and match
// anything.
func MatchAsset(name, osName, arch string) bool {
	if osName == "" || arch == "" || osName == "all" {
		return true
	}
	lower := strings.ToLower(name)
	o := strings.ToLower(osName)
	a := strings.ToLower(arch)

	// common arch aliases
	osAliases := map[string][]string{"macos": {"darwin", "osx"}, "osx": {"darwin", "macos"}}
	archAliases := map[string][]string{
		"amd64": {"x86_64", "x64"},
		"arm64": {"aarch64"},
		"386":   {"i386", "i686", "x86"},
	}

	osNames := append([]string{o}, osAliases[o]...)
	archNames := append([]string{a}, archAliases[a]...)

	for _, sep := range []string{"-", "_", "."} {
		for _, on := range osNames {
			for _, an := range archNames {
				if strings.Contains(lower, on+sep+an) || strings.Contains(lower, an+sep+on) {
					return true
				}
			}
		}
	}
	for _, on := range osNames {
		for _, an := range archNames {
			if strings.Contains(lower, on) && strings.Contains(lower, an) {
				return true
			}
		}
	}
	// neutral asset: matches only if it mentions no known platform marker
	return !hasPlatformMarker(lower)
}

var platformMarkers = []string{
	"linux", "windows", "darwin", "macos", "freebsd",
	"amd64", "x86_64", "x64", "arm64", "aarch64", "386", "i386", "armv7", "arm",
}

func hasPlatformMarker(s string) bool {
	for _, m := range platformMarkers {
		if strings.Contains(s, m) {
			return true
		}
	}
	return false
}
