// Command updatelib builds the C ABI shared library (DLL/so/dylib) so host
// applications can embed update functionality in-process.
//
// Build:
//
//	make lib     # dist/libupdate.so|.dylib|.dll + dist/libupdate.h
//
// ABI contract (see dist/libupdate.h):
//   - update_check / update_download / update_list / update_version return a
//     C string allocated with C.malloc; the caller MUST release it with
//     update_free.
//   - On failure they return NULL and the message is available via
//     update_last_error (static buffer, no free needed).
//   - All functions are thread-safe except that update_last_error is a
//     process-global last-error slot (callers using multiple threads should
//     guard it themselves).
package main

/*
#include <stdlib.h>
*/
import "C"

import (
	"strings"
	"sync"
	"unsafe"

	"update/internal/lib"
	"update/internal/versioninfo"
)

func main() {}

// lastError is a process-global slot for the most recent failure message.
var lastError struct {
	sync.Mutex
	msg [4096]byte
}

func setLastError(err error) {
	lastError.Lock()
	defer lastError.Unlock()
	msg := err.Error()
	if len(msg) > len(lastError.msg)-1 {
		msg = msg[:len(lastError.msg)-1]
	}
	copy(lastError.msg[:], msg)
	lastError.msg[len(msg)] = 0
}

// cstr converts a C string to a Go string, tolerating NULL.
func cstr(p *C.char) string {
	if p == nil {
		return ""
	}
	return C.GoString(p)
}

// exportString hands a Go string to C (C.malloc + copy). Returns NULL on
// allocation failure (never in practice for small payloads).
func exportString(s string) *C.char {
	c := C.CString(s)
	return c
}

// goArgs builds argv for lib.RunCommand skipping empty values.
func goArgs(flags ...string) []string {
	args := make([]string, 0, len(flags)/2+1)
	args = append(args, flags[0])
	for i := 1; i+1 < len(flags); i += 2 {
		if flags[i+1] != "" {
			args = append(args, flags[i], flags[i+1])
		}
	}
	return args
}

//export update_check
func update_check(configPath, currentVersion, platform, username, password *C.char) *C.char {
	out, err := lib.RunCommand(goArgs("check",
		"--config", cstr(configPath),
		"--current-version", cstr(currentVersion),
		"--platform", cstr(platform),
		"--username", cstr(username),
		"--password", cstr(password))...)
	if err != nil {
		setLastError(err)
		return nil
	}
	return exportString(out)
}

//export update_download
func update_download(configPath, version, asset, outPath, platform, username, password *C.char) *C.char {
	out, err := lib.RunCommand(goArgs("download",
		"--config", cstr(configPath),
		"--version", cstr(version),
		"--asset", cstr(asset),
		"--out", cstr(outPath),
		"--platform", cstr(platform),
		"--username", cstr(username),
		"--password", cstr(password))...)
	if err != nil {
		setLastError(err)
		return nil
	}
	return exportString(out)
}

//export update_list
func update_list(configPath, platform, username, password *C.char, limit C.int) *C.char {
	out, err := lib.RunCommand(goArgs("list",
		"--config", cstr(configPath),
		"--limit", limitString(int(limit)),
		"--platform", cstr(platform),
		"--username", cstr(username),
		"--password", cstr(password))...)
	if err != nil {
		setLastError(err)
		return nil
	}
	return exportString(out)
}

func limitString(n int) string {
	if n <= 0 {
		return "10"
	}
	return itoa(n)
}

// itoa is a tiny int -> string (avoids fmt dependency churn; fine here).
func itoa(n int) string {
	if n == 0 {
		return "0"
	}
	neg := n < 0
	if neg {
		n = -n
	}
	var b [12]byte
	i := len(b)
	for n > 0 {
		i--
		b[i] = byte('0' + n%10)
		n /= 10
	}
	if neg {
		i--
		b[i] = '-'
	}
	return string(b[i:])
}

//export update_version
func update_version() *C.char {
	return exportString(versioninfo.String())
}

//export update_last_error
func update_last_error() *C.char {
	lastError.Lock()
	defer lastError.Unlock()
	// Find the NUL terminator and return a pointer into the static buffer.
	msg := lastError.msg[:]
	_ = strings.IndexByte
	end := 0
	for end < len(msg) && msg[end] != 0 {
		end++
	}
	if end == 0 {
		return nil
	}
	return (*C.char)(unsafe.Pointer(&msg[0]))
}

//export update_free
func update_free(ptr *C.char) {
	if ptr == nil {
		return
	}
	C.free(unsafe.Pointer(ptr))
}
