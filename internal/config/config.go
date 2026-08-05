// Package config loads and validates the update module configuration.
// Secrets (tokens) are never stored in the file — they are injected from
// environment variables named by token_env / username_env fields.
package config

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
)

// Config is the root configuration document.
type Config struct {
	Product Product      `json:"product"`
	Source  SourceConfig `json:"source"`
}

// Product identifies the host application and its current version.
type Product struct {
	Name           string `json:"name"`
	CurrentVersion string `json:"current_version"`
	AssetFilter    string `json:"asset_filter,omitempty"` // regex, optional
}

// SourceConfig selects the release source.
type SourceConfig struct {
	Type   string        `json:"type"` // "github-tag" | "custom"
	GitHub *GitHubConfig `json:"github,omitempty"`
	Custom *CustomConfig `json:"custom,omitempty"`
}

// GitHubConfig configures a GitHub tag/release source.
type GitHubConfig struct {
	Owner    string `json:"owner"`
	Repo     string `json:"repo"`
	TokenEnv string `json:"token_env,omitempty"` // env var name holding a PAT (or password for basic auth)
	// UsernameEnv enables HTTP Basic authentication (username + token/password),
	// required by some internal GitHub Enterprise instances.
	UsernameEnv string `json:"username_env,omitempty"`
	APIBaseURL  string `json:"api_base_url,omitempty"` // default https://api.github.com (GitHub Enterprise override)
	UseReleases bool   `json:"use_releases"`           // true: releases+assets; false: tags only

	Token    string `json:"-"`
	Username string `json:"-"`
}

// CustomConfig configures a custom HTTP source.
type CustomConfig struct {
	VersionsURL         string            `json:"versions_url"`
	Headers             map[string]string `json:"headers,omitempty"`
	Auth                *AuthConfig       `json:"auth,omitempty"`
	DownloadURLTemplate string            `json:"download_url_template,omitempty"` // supports {version} and {asset}
}

// AuthConfig describes HTTP authentication for a custom source.
type AuthConfig struct {
	Type        string `json:"type"` // "bearer" | "basic"
	TokenEnv    string `json:"token_env,omitempty"`
	UsernameEnv string `json:"username_env,omitempty"`

	Token    string `json:"-"`
	Username string `json:"-"`
}

const defaultGitHubAPIBaseURL = "https://api.github.com"

// DefaultGitHubAPIBaseURL returns the default GitHub API base URL.
func DefaultGitHubAPIBaseURL() string { return defaultGitHubAPIBaseURL }

// Load reads the config file at path, parses it, applies defaults and
// resolves secrets from the environment. getenv is injectable for tests.
func Load(path string, getenv func(string) string) (*Config, error) {
	if getenv == nil {
		getenv = os.Getenv
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read config: %w", err)
	}
	return Parse(data, getenv)
}

// Parse parses config bytes, applies defaults and resolves secrets.
func Parse(data []byte, getenv func(string) string) (*Config, error) {
	cfg := &Config{}
	if err := json.Unmarshal(data, cfg); err != nil {
		return nil, fmt.Errorf("parse config: %w", err)
	}
	if err := cfg.validateAndResolve(getenv); err != nil {
		return nil, err
	}
	return cfg, nil
}

func (c *Config) validateAndResolve(getenv func(string) string) error {
	switch c.Source.Type {
	case "github-tag":
		g := c.Source.GitHub
		if g == nil {
			return errors.New("config: source type \"github-tag\" requires a github section")
		}
		if g.Owner == "" || g.Repo == "" {
			return errors.New("config: github.owner and github.repo are required")
		}
		if g.APIBaseURL == "" {
			g.APIBaseURL = defaultGitHubAPIBaseURL
		}
		if g.TokenEnv != "" {
			g.Token = getenv(g.TokenEnv)
		}
		if g.UsernameEnv != "" {
			g.Username = getenv(g.UsernameEnv)
		}
	case "custom":
		cu := c.Source.Custom
		if cu == nil {
			return errors.New("config: source type \"custom\" requires a custom section")
		}
		if cu.VersionsURL == "" {
			return errors.New("config: custom.versions_url is required")
		}
		if cu.Auth != nil {
			switch cu.Auth.Type {
			case "bearer":
				if cu.Auth.TokenEnv != "" {
					cu.Auth.Token = getenv(cu.Auth.TokenEnv)
				}
			case "basic":
				if cu.Auth.UsernameEnv != "" {
					cu.Auth.Username = getenv(cu.Auth.UsernameEnv)
				}
				if cu.Auth.TokenEnv != "" {
					cu.Auth.Token = getenv(cu.Auth.TokenEnv)
				}
			default:
				return fmt.Errorf("config: unsupported auth type %q (want \"bearer\" or \"basic\")", cu.Auth.Type)
			}
		}
	default:
		return fmt.Errorf("config: unsupported source type %q (want \"github-tag\" or \"custom\")", c.Source.Type)
	}
	return nil
}
