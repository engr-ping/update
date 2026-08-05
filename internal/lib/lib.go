// Package lib exposes the update core as a Go API. It reuses the CLI layer
// (single source of truth: CLI and C ABI behave identically) by capturing
// stdout/stderr.
package lib

import (
	"bytes"
	"context"
	"fmt"
	"strings"

	"update/internal/cli"
)

// RunCommand executes an update subcommand and returns its stdout as a
// string. On failure (exit code != 0) it returns an error containing the
// stderr output.
func RunCommand(args ...string) (string, error) {
	var stdout, stderr bytes.Buffer
	code := cli.Run(context.Background(), args, &stdout, &stderr)
	if code != 0 {
		return "", fmt.Errorf("exit %d: %s", code, strings.TrimSpace(stderr.String()))
	}
	return stdout.String(), nil
}

// Check runs "update check" and returns the JSON result.
func Check(configPath, currentVersion, platform, username, password string) (string, error) {
	args := []string{"check"}
	appendFlag(&args, "--config", configPath)
	appendFlag(&args, "--current-version", currentVersion)
	appendFlag(&args, "--platform", platform)
	appendFlag(&args, "--username", username)
	appendFlag(&args, "--password", password)
	return RunCommand(args...)
}

// Download runs "update download" and returns the JSON result.
func Download(configPath, version, asset, outPath, platform, username, password string) (string, error) {
	args := []string{"download"}
	appendFlag(&args, "--config", configPath)
	appendFlag(&args, "--version", version)
	appendFlag(&args, "--asset", asset)
	appendFlag(&args, "--out", outPath)
	appendFlag(&args, "--platform", platform)
	appendFlag(&args, "--username", username)
	appendFlag(&args, "--password", password)
	return RunCommand(args...)
}

// List runs "update list" and returns the JSON result.
func List(configPath, platform, username, password string, limit int) (string, error) {
	args := []string{"list"}
	appendFlag(&args, "--config", configPath)
	if limit > 0 {
		args = append(args, "--limit", fmt.Sprintf("%d", limit))
	}
	appendFlag(&args, "--platform", platform)
	appendFlag(&args, "--username", username)
	appendFlag(&args, "--password", password)
	return RunCommand(args...)
}

// Version returns the update library version string.
func Version() string {
	out, err := RunCommand("version")
	if err != nil {
		return "dev"
	}
	return strings.TrimSpace(out)
}

func appendFlag(args *[]string, name, value string) {
	if value != "" {
		*args = append(*args, name, value)
	}
}
