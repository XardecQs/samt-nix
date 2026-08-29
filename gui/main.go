package main

import (
	"fmt"
	"os"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/widget"
)

type GUI struct {
	bin string
	win fyne.Window

	profiles []ProfileInfo
	active   string

	selectedProfile int

	profileSelect *widget.Select
	modsBox       *fyne.Container
	profilesBox   *fyne.Container
	status        *widget.Label

	actionUse      *widget.Button
	actionRename   *widget.Button
	actionCopy     *widget.Button
	actionDelete   *widget.Button

	mods []Mod
}

func main() {
	bin, err := findBinary()
	if err != nil {
		fmt.Fprintln(os.Stderr, "gta-mo-gui:", err)
		os.Exit(1)
	}

	a := app.New()
	g := &GUI{bin: bin, selectedProfile: -1}

	w := a.NewWindow("GTA Mod Organizer")
	g.win = w
	w.Resize(fyne.NewSize(760, 560))

	w.SetContent(g.buildUI())
	g.refreshProfiles()
	w.ShowAndRun()
}

func (g *GUI) buildUI() fyne.CanvasObject {
	g.profileSelect = widget.NewSelect([]string{}, func(slug string) {
		g.selectProfile(slug)
	})

	g.status = widget.NewLabel("")
	g.status.Wrapping = fyne.TextWrapWord

	launchBtn := widget.NewButton("Lanzar", g.launch)
	refreshBtn := widget.NewButton("Actualizar", g.refreshProfiles)

	top := container.NewBorder(
		nil,
		nil,
		container.NewHBox(
			widget.NewLabel("Perfil:"),
			g.profileSelect,
			refreshBtn,
		),
		launchBtn,
	)

	modsTab := container.NewBorder(
		container.NewHBox(
			widget.NewLabel("Mods del perfil activo:"),
			widget.NewButton("Nuevo mod", func() { g.addMod() }),
		),
		nil, nil, nil,
		container.NewVScroll(g.modsTab()),
	)

	profilesTab := g.profilesTab()

	tabs := container.NewAppTabs(
		container.NewTabItem("Mods", modsTab),
		container.NewTabItem("Perfiles", profilesTab),
	)

	return container.NewBorder(
		container.NewVBox(
			top,
			widget.NewSeparator(),
		),
		container.NewVBox(
			widget.NewSeparator(),
			container.NewPadded(g.status),
		),
		nil, nil,
		tabs,
	)
}

// ---------- Mods tab ----------

func (g *GUI) modsTab() fyne.CanvasObject {
	g.modsBox = container.NewVBox()
	return g.modsBox
}

func (g *GUI) modRow(m Mod) fyne.CanvasObject {
	label := m.Name
	if m.Folder != "" && m.Folder != m.Name {
		label = fmt.Sprintf("%s  (%s)", m.Name, m.Folder)
	}
	if len(m.Deps) > 0 {
		var deps []string
		for _, d := range m.Deps {
			name := d.Name
			if !d.Required {
				name += " (opc)"
			}
			deps = append(deps, name)
		}
		label += "\n    depende de: " + strings.Join(deps, ", ")
	}

	check := widget.NewCheck(label, nil)
	check.SetChecked(m.Enabled)
	check.OnChanged = func(on bool) {
		if on == m.Enabled {
			return
		}
		g.toggleMod(m, on)
	}

	up := widget.NewButton("▲", func() { g.moveMod(m, -1) })
	down := widget.NewButton("▼", func() { g.moveMod(m, 1) })
	order := widget.NewLabel(fmt.Sprintf("%d", m.Order))

	return container.NewBorder(
		nil, nil,
		container.NewHBox(up, down, order),
		nil,
		container.NewPadded(check),
	)
}

func (g *GUI) refreshMods() {
	if g.active == "" {
		return
	}
	var mods []Mod
	if err := runJSON(g.bin, &mods, "ctl", "list", "--json", "--profile", g.active); err != nil {
		dialog.ShowError(err, g.win)
		return
	}
	g.mods = mods
	g.modsBox.Objects = g.modsBox.Objects[:0]
	for _, m := range mods {
		g.modsBox.Add(g.modRow(m))
	}
	g.modsBox.Refresh()
}

func (g *GUI) toggleMod(m Mod, on bool) {
	verb := "disable"
	args := []string{"ctl", verb, fmt.Sprint(m.ID), "--profile", g.active}
	if !on {
		args = []string{"ctl", "disable", fmt.Sprint(m.ID), "--yes", "--profile", g.active}
	}
	if _, err := run(g.bin, args...); err != nil {
		dialog.ShowError(err, g.win)
		g.refreshMods()
		return
	}
	g.refreshMods()
}

func (g *GUI) moveMod(m Mod, dir int) {
	idx := -1
	for i, x := range g.mods {
		if x.ID == m.ID {
			idx = i
			break
		}
	}
	ni := idx + dir
	if idx < 0 || ni < 0 || ni >= len(g.mods) {
		return
	}
	other := g.mods[ni]
	if _, err := run(g.bin, "ctl", "order", fmt.Sprint(m.ID), fmt.Sprint(other.Order), "--profile", g.active); err != nil {
		dialog.ShowError(err, g.win)
		return
	}
	if _, err := run(g.bin, "ctl", "order", fmt.Sprint(other.ID), fmt.Sprint(m.Order), "--profile", g.active); err != nil {
		dialog.ShowError(err, g.win)
		return
	}
	g.refreshMods()
}

func (g *GUI) addMod() {
	dialog.ShowEntryDialog("Nuevo mod", "Nombre de carpeta:", func(folder string) {
		folder = strings.TrimSpace(folder)
		if folder == "" {
			return
		}
		if _, err := run(g.bin, "ctl", "add", folder); err != nil {
			dialog.ShowError(err, g.win)
			return
		}
		g.refreshMods()
	}, g.win)
}

// ---------- Profiles tab ----------

func (g *GUI) profilesTab() fyne.CanvasObject {
	g.profilesBox = container.NewVBox()

	g.actionUse = widget.NewButton("Usar", g.useSelectedProfile)
	g.actionRename = widget.NewButton("Renombrar", g.renameSelectedProfile)
	g.actionCopy = widget.NewButton("Copiar", g.copySelectedProfile)
	g.actionDelete = widget.NewButton("Eliminar", g.deleteSelectedProfile)
	newBtn := widget.NewButton("Nuevo", g.newProfile)

	actions := container.NewHBox(newBtn, g.actionUse, g.actionRename, g.actionCopy, g.actionDelete)

	return container.NewBorder(
		container.NewPadded(actions),
		nil, nil, nil,
		container.NewVScroll(g.profilesBox),
	)
}

func (g *GUI) refreshProfilesTable() {
	g.profilesBox.Objects = g.profilesBox.Objects[:0]
	hasSelection := false
	for i, p := range g.profiles {
		active := ""
		if p.Active {
			active = "  [activo]"
		}
		label := fmt.Sprintf("%d. %s%s  —  %d mods, %d activos", p.ID, p.Name, active, p.Mods, p.Enabled)
		btn := widget.NewButton(label, func(i int) func() {
			return func() {
				g.selectedProfile = i
				g.refreshProfilesTable()
			}
		}(i))
		if i == g.selectedProfile || p.Active {
			btn.Importance = widget.HighImportance
			hasSelection = true
		}
		g.profilesBox.Add(btn)
	}
	if g.selectedProfile == -1 && hasSelection {
		// fallback: nothing selected; find the active one for actions
		for i, p := range g.profiles {
			if p.Active {
				g.selectedProfile = i
				break
			}
		}
	}
	g.profilesBox.Refresh()
	g.updateProfileActions()
}

func (g *GUI) updateProfileActions() {
	has := g.selectedProfile >= 0 && g.selectedProfile < len(g.profiles)
	g.actionUse.Disable()
	g.actionRename.Disable()
	g.actionCopy.Disable()
	g.actionDelete.Disable()
	if has {
		g.actionUse.Enable()
		g.actionRename.Enable()
		g.actionCopy.Enable()
		g.actionDelete.Enable()
	}
}

func (g *GUI) selected() *ProfileInfo {
	if g.selectedProfile < 0 || g.selectedProfile >= len(g.profiles) {
		return nil
	}
	return &g.profiles[g.selectedProfile]
}

func (g *GUI) newProfile() {
	dialog.ShowEntryDialog("Nuevo perfil", "Nombre:", func(name string) {
		name = strings.TrimSpace(name)
		if name == "" {
			return
		}
		if _, err := run(g.bin, "ctl", "profile", "create", name); err != nil {
			dialog.ShowError(err, g.win)
			return
		}
		g.refreshProfiles()
	}, g.win)
}

func (g *GUI) useSelectedProfile() {
	p := g.selected()
	if p == nil {
		return
	}
	if _, err := run(g.bin, "ctl", "profile", "use", p.Slug); err != nil {
		dialog.ShowError(err, g.win)
		return
	}
	g.refreshProfiles()
}

func (g *GUI) renameSelectedProfile() {
	p := g.selected()
	if p == nil {
		return
	}
	dialog.ShowEntryDialog("Renombrar perfil", "Nuevo nombre:", func(name string) {
		name = strings.TrimSpace(name)
		if name == "" {
			return
		}
		if _, err := run(g.bin, "ctl", "profile", "rename", p.Slug, name); err != nil {
			dialog.ShowError(err, g.win)
			return
		}
		g.refreshProfiles()
	}, g.win)
}

func (g *GUI) copySelectedProfile() {
	p := g.selected()
	if p == nil {
		return
	}
	dialog.ShowEntryDialog("Copiar perfil", "Nombre del nuevo perfil:", func(name string) {
		name = strings.TrimSpace(name)
		if name == "" {
			return
		}
		if _, err := run(g.bin, "ctl", "profile", "copy", p.Slug, name); err != nil {
			dialog.ShowError(err, g.win)
			return
		}
		g.refreshProfiles()
	}, g.win)
}

func (g *GUI) deleteSelectedProfile() {
	p := g.selected()
	if p == nil {
		return
	}
	dialog.ShowConfirm(
		"Eliminar perfil",
		fmt.Sprintf("¿Eliminar el perfil '%s'?\nSe borrarán sus estados y su directorio en run/profiles/.", p.Name),
		func(ok bool) {
			if !ok {
				return
			}
			if _, err := run(g.bin, "ctl", "profile", "delete", p.Slug, "--yes"); err != nil {
				dialog.ShowError(err, g.win)
				return
			}
			g.selectedProfile = -1
			g.refreshProfiles()
		},
		g.win,
	)
}

// ---------- Global ----------

func (g *GUI) refreshProfiles() {
	var profiles []ProfileInfo
	if err := runJSON(g.bin, &profiles, "ctl", "profile", "list", "--json"); err != nil {
		dialog.ShowError(err, g.win)
		return
	}
	g.profiles = profiles

	opts := make([]string, 0, len(profiles))
	active := ""
	for _, p := range profiles {
		opts = append(opts, p.Slug)
		if p.Active {
			active = p.Slug
		}
	}

	prev := g.profileSelect.OnChanged
	g.profileSelect.OnChanged = nil
	g.profileSelect.Options = opts
	g.profileSelect.SetSelected(active)
	g.profileSelect.OnChanged = prev

	g.active = active
	g.refreshProfilesTable()
	g.refreshMods()
}

func (g *GUI) selectProfile(slug string) {
	if slug == "" {
		return
	}
	if _, err := run(g.bin, "ctl", "profile", "use", slug); err != nil {
		dialog.ShowError(err, g.win)
		return
	}
	g.active = slug
	g.refreshProfiles()
}

func (g *GUI) launch() {
	if g.active == "" {
		return
	}
	bin := g.bin
	profile := g.active
	g.status.SetText("Lanzando…")
	go func() {
		out, err := run(bin, "launch", "--deps-enable", "--profile", profile)
		fyne.Do(func() {
			if err != nil {
				g.status.SetText("Error al lanzar")
				dialog.ShowError(err, g.win)
				return
			}
			text := "Juego terminado."
			if strings.TrimSpace(out) != "" {
				text += "\n" + out
			}
			g.status.SetText(text)
		})
	}()
}
