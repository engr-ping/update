// Command update is a language-agnostic software update module.
//
// Host applications invoke it as a subprocess: arguments in, JSON on stdout,
// logs/errors on stderr, exit codes classify failures. See docs/design.md.
package main

import (
	"context"
	"os"

	"update/internal/cli"
)

func main() {
	os.Exit(cli.Run(context.Background(), os.Args[1:], os.Stdout, os.Stderr))
}
