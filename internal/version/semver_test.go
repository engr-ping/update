package version

import "testing"

func TestCompare(t *testing.T) {
	tests := []struct {
		a, b string
		want int
	}{
		{"1.0.0", "1.0.0", 0},
		{"v1.0.0", "1.0.0", 0},
		{"1.0.0", "1.0.1", -1},
		{"1.0.1", "1.0.0", 1},
		{"1.2.0", "1.10.0", -1},
		{"1.10.0", "1.2.0", 1},
		{"2.0.0", "1.99.99", 1},
		{"1.0.0-alpha", "1.0.0", -1},
		{"1.0.0", "1.0.0-alpha", 1},
		{"1.0.0-alpha", "1.0.0-alpha", 0},
		{"1.0.0-alpha.1", "1.0.0-alpha.2", -1},
		{"1.0.0-rc.1", "1.0.0-beta.1", 1},
		{"1.0.0-1", "1.0.0-alpha", -1}, // numeric < alphanumeric
		{"1.0.0+build.1", "1.0.0", 0},  // build metadata ignored
		{"v1.2.3", "1.2.4", -1},
		{"release-1.2.0", "1.2.0", -1}, // release- is not stripped by Compare
		{"abc", "def", -1},             // non-semver lexical
		{"1.2", "1.2.0", 0},            // two-part accepted
		{"", "1.0.0", -1},
	}
	for _, tt := range tests {
		got := Compare(tt.a, tt.b)
		if got != tt.want {
			t.Errorf("Compare(%q, %q) = %d, want %d", tt.a, tt.b, got, tt.want)
		}
	}
}

func TestCleanTag(t *testing.T) {
	tests := []struct{ in, want string }{
		{"v1.2.3", "1.2.3"},
		{"V1.2.3", "1.2.3"},
		{"release-1.2.0", "1.2.0"},
		{"1.2.3", "1.2.3"},
	}
	for _, tt := range tests {
		if got := CleanTag(tt.in); got != tt.want {
			t.Errorf("CleanTag(%q) = %q, want %q", tt.in, got, tt.want)
		}
	}
}

func TestIsSemver(t *testing.T) {
	tests := []struct {
		in   string
		want bool
	}{
		{"1.2.3", true},
		{"v1.2.3", true},
		{"1.2.3-rc.1", true},
		{"1.2", true},
		{"abc", false},
		{"", false},
		{"1.2.x", false},
	}
	for _, tt := range tests {
		if got := IsSemver(tt.in); got != tt.want {
			t.Errorf("IsSemver(%q) = %v, want %v", tt.in, got, tt.want)
		}
	}
}
