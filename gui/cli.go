package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type Mod struct {
	ID         int64   `json:"id"`
	Folder     string  `json:"folder"`
	Name       string  `json:"name"`
	Enabled    bool    `json:"enabled"`
	Order      int64   `json:"order"`
	Deps       []Dep   `json:"deps,omitempty"`
	Dependents []int64 `json:"dependents,omitempty"`
}

type Dep struct {
	ID       int64  `json:"id"`
	Folder   string `json:"folder"`
	Name     string `json:"name"`
	Required bool   `json:"required"`
}

type ProfileInfo struct {
	ID      int64  `json:"id"`
	Name    string `json:"name"`
	Slug    string `json:"slug"`
	Active  bool   `json:"active"`
	Mods    int64  `json:"mods"`
	Enabled int64  `json:"enabled"`
}

func findBinary() (string, error) {
	if p := os.Getenv("GTA_MO_BIN"); p != "" {
		if _, err := os.Stat(p); err == nil {
			return p, nil
		}
		return "", fmt.Errorf("GTA_MO_BIN apunta a %q pero no existe", p)
	}
	if p, err := exec.LookPath("gta-mo"); err == nil {
		return p, nil
	}

	cands := []string{
		"../target/debug/gta-mo",
		"../../target/debug/gta-mo",
		"target/debug/gta-mo",
	}
	if exe, err := os.Executable(); err == nil {
		dir := filepath.Dir(exe)
		for _, rel := range []string{
			"../target/debug/gta-mo",
			"../../target/debug/gta-mo",
			"../../../target/debug/gta-mo",
		} {
			cands = append(cands, filepath.Join(dir, rel))
		}
	}
	for _, c := range cands {
		if fi, err := os.Stat(c); err == nil && !fi.IsDir() {
			abs, _ := filepath.Abs(c)
			return abs, nil
		}
	}
	return "", fmt.Errorf("no se encontró el binario gta-mo; defínelo con GTA_MO_BIN")
}

// run executes gta-mo with the given args and returns combined output.
func run(bin string, args ...string) (string, error) {
	cmd := exec.Command(bin, args...)
	var out, stderr bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = &stderr
	err := cmd.Run()

	combined := strings.TrimSpace(out.String())
	if e := strings.TrimSpace(stderr.String()); e != "" {
		if combined != "" {
			combined += "\n" + e
		} else {
			combined = e
		}
	}
	if err != nil {
		if combined == "" {
			combined = err.Error()
		}
		return combined, fmt.Errorf("%s", combined)
	}
	return combined, nil
}

// runJSON executes gta-mo and decodes its stdout as JSON.
func runJSON(bin string, out interface{}, args ...string) error {
	cmd := exec.Command(bin, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = err.Error()
		}
		return fmt.Errorf("%s", msg)
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(stdout.Bytes(), out)
}
