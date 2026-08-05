// Package version implements semver comparison, version tag parsing and
// platform/asset matching. It is a zero-dependency reimplementation of the
// parts of semver 2.0 we need (no third-party libs allowed in this project).
package version

import (
	"strconv"
	"strings"
)

// semver holds the parsed components of a semantic version string.
type semver struct {
	core   [3]int64 // major, minor, patch
	pre    []string // pre-release identifiers (without '-')
	hasPre bool
}

// Compare compares two version strings and returns -1, 0 or +1 if a is
// less than, equal to, or greater than b.
//
// Rules:
//   - optional "v"/"V" prefix is ignored
//   - build metadata after '+' is ignored for comparison
//   - a missing pre-release sorts after one that has it (1.0.0 > 1.0.0-rc1)
//   - pre-release identifiers: numeric identifiers compare numerically,
//     alphanumeric compare lexically, numeric sorts before alphanumeric
//   - two-part versions like "1.2" are accepted (patch treated as 0)
//   - non-semver strings fall back to case-insensitive lexical comparison
func Compare(a, b string) int {
	sa, okA := parse(a)
	sb, okB := parse(b)
	switch {
	case okA && okB:
		return compareSemver(sa, sb)
	case !okA && !okB:
		return strings.Compare(strings.ToLower(a), strings.ToLower(b))
	case !okA:
		return -1 // non-semver sorts below semver
	default:
		return 1
	}
}

// parse parses a version string into a semver struct.
func parse(s string) (semver, bool) {
	s = strings.TrimSpace(s)
	s = strings.TrimPrefix(s, "v")
	s = strings.TrimPrefix(s, "V")
	if s == "" {
		return semver{}, false
	}
	// drop build metadata
	if i := strings.IndexByte(s, '+'); i >= 0 {
		s = s[:i]
	}
	var pre string
	if i := strings.IndexByte(s, '-'); i >= 0 {
		pre = s[i+1:]
		s = s[:i]
	}
	if s == "" {
		return semver{}, false
	}

	parts := strings.Split(s, ".")
	if len(parts) < 2 || len(parts) > 3 {
		return semver{}, false
	}
	sv := semver{}
	for i, p := range parts {
		if p == "" {
			return semver{}, false
		}
		n, err := strconv.ParseInt(p, 10, 64)
		if err != nil {
			return semver{}, false
		}
		sv.core[i] = n
	}
	if pre != "" {
		ids := strings.Split(pre, ".")
		for _, id := range ids {
			if id == "" {
				return semver{}, false
			}
		}
		sv.pre = ids
		sv.hasPre = true
	}
	return sv, true
}

func compareSemver(a, b semver) int {
	for i := 0; i < 3; i++ {
		if a.core[i] < b.core[i] {
			return -1
		}
		if a.core[i] > b.core[i] {
			return 1
		}
	}
	switch {
	case !a.hasPre && !b.hasPre:
		return 0
	case a.hasPre && !b.hasPre:
		return -1
	case !a.hasPre && b.hasPre:
		return 1
	}
	// both have pre-release identifiers
	for i := 0; i < len(a.pre) && i < len(b.pre); i++ {
		if c := comparePreID(a.pre[i], b.pre[i]); c != 0 {
			return c
		}
	}
	switch {
	case len(a.pre) < len(b.pre):
		return -1
	case len(a.pre) > len(b.pre):
		return 1
	}
	return 0
}

// comparePreID compares two pre-release identifiers per semver 2.0.
func comparePreID(a, b string) int {
	ai, aErr := strconv.ParseInt(a, 10, 64)
	bi, bErr := strconv.ParseInt(b, 10, 64)
	switch {
	case aErr == nil && bErr == nil:
		if ai < bi {
			return -1
		}
		if ai > bi {
			return 1
		}
		return 0
	case aErr == nil:
		return -1 // numeric sorts before alphanumeric
	case bErr == nil:
		return 1
	default:
		return strings.Compare(a, b)
	}
}

// IsSemver reports whether s can be parsed as a semantic version.
func IsSemver(s string) bool {
	_, ok := parse(s)
	return ok
}
