package main

import (
	"strings"
	"testing"
)

func TestToggleArgs(t *testing.T) {
	m := Mod{ID: 42}
	enable := toggleArgs(m, true, "p1")
	if len(enable) < 3 || enable[1] != "enable" {
		t.Fatalf("enable args incorrectos: %v", enable)
	}
	for _, a := range enable {
		if a == "--yes" {
			t.Fatalf("enable no debería llevar --yes: %v", enable)
		}
	}

	disable := toggleArgs(m, false, "p1")
	if len(disable) < 3 || disable[1] != "disable" {
		t.Fatalf("disable args incorrectos: %v", disable)
	}
	hasYes := false
	for _, a := range disable {
		if a == "--yes" {
			hasYes = true
		}
	}
	if !hasYes {
		t.Fatalf("disable debería llevar --yes: %v", disable)
	}
}

func TestRunStream(t *testing.T) {
	var lines []string
	err := runStream("sh", []string{"-c", "printf 'a\\nb\\nc\\n'"}, func(s string) {
		lines = append(lines, s)
	})
	if err != nil {
		t.Fatalf("runStream error: %v", err)
	}
	if strings.Join(lines, ",") != "a,b,c" {
		t.Fatalf("líneas esperadas a,b,c; got %v", lines)
	}
}
