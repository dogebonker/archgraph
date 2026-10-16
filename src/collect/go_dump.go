// archgraph's Go collector helper, run by `go run go_dump.go <root> <go> <module dir>...` from a
// temporary directory. Standard library only. It lists the local packages of every module under
// <root> with `go list -find` (which loads no dependencies and builds nothing), type-checks them
// with go/types (the standard library through go/importer, local packages from source) without
// running or compiling repository code, and prints every reference from one file to another as
// JSON on stdout. Paths are relative to <root>, slash-separated.
package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"go/ast"
	"go/build"
	"go/build/constraint"
	"go/importer"
	"go/parser"
	"go/token"
	"go/types"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
)

// The environment of every go command: no toolchain download (a go.mod `toolchain` line is
// ignored, a `go` line newer than this toolchain fails), no network, no workspace file, no cgo,
// and GOFLAGS replaced, so no -toolexec or -overlay from the user's go env applies.
var safeEnv = []string{"GOTOOLCHAIN=local", "GOPROXY=off", "GOWORK=off", "CGO_ENABLED=0"}

type listed struct {
	ImportPath, Name, Dir                                        string
	Standard                                                     bool
	Module                                                       *struct{ Path string }
	GoFiles, CgoFiles, IgnoredGoFiles, TestGoFiles, XTestGoFiles []string
}

type pkg struct {
	path, name, dir string
	files           []*ast.File
	// Top-level names (`F`, `T.M` for methods) to every file declaring them: one name declared
	// in several platform files (foo_linux.go, foo_windows.go) maps to all of them.
	decls       map[string][]string
	testSupport bool // by its name: *testing, *testutil
	types       *types.Package
	busy        bool
}

type fileOut struct {
	Path      string `json:"path"`
	Generated bool   `json:"generated"`
	Main      bool   `json:"main,omitempty"`
	// A file of a test-support package: one named *testing or *testutil, or whose non-test
	// files import "testing".
	TestSupport bool `json:"test_support,omitempty"`
}

type edge struct {
	From string `json:"from"`
	To   string `json:"to"`
}

var (
	root  string
	fset  = token.NewFileSet()
	local = map[string]*pkg{}
	// Local packages and external test packages (`foo_test`) by import path.
	byPath  = map[string]*pkg{}
	std     types.Importer
	modules = map[string]bool{}
	// Pairs of files of one package that cannot be split across packages and still compile:
	// a method and its type's file, and a file using an unexported name and the file declaring
	// it.
	bound = map[edge]bool{}
	// The package of each file.
	byFile = map[string]*pkg{}
)

func main() {
	args := os.Args[1:]
	if len(args) < 3 {
		fail(errors.New("usage: go_dump <root> <go> <module dir>..."))
	}
	// The go command reports directories with symbolic links resolved.
	abs, err := filepath.Abs(args[0])
	if err == nil {
		root, err = filepath.EvalSymlinks(abs)
	}
	if err != nil {
		fail(err)
	}
	gobin := args[1]
	std = importer.ForCompiler(fset, "source", nil)

	// The module directories, root-relative: archgraph finds them (honouring --exclude).
	rels := args[2:]
	sort.Strings(rels)
	var dirs []string
	for _, r := range rels {
		dirs = append(dirs, filepath.Join(root, filepath.FromSlash(r)))
	}
	var listedPkgs []listed
	for i, dir := range dirs {
		pkgs, err := list(gobin, dir)
		if err != nil {
			// A Go too old for the module is fatal: its sources may not even parse.
			if strings.Contains(err.Error(), "requires go >=") {
				fail(err)
			}
			lines := strings.Split(strings.TrimSpace(err.Error()), "\n")
			fmt.Fprintf(os.Stderr, "warning: go list failed in %s (%s); reading its packages with go/build\n",
				rels[i], strings.TrimSpace(lines[len(lines)-1]))
			pkgs = buildList(dir)
		}
		listedPkgs = append(listedPkgs, pkgs...)
	}

	var xtests []*pkg
	for _, l := range listedPkgs {
		if l.Standard || !inside(l.Dir) || local[l.ImportPath] != nil {
			continue
		}
		if l.Module != nil {
			modules[l.Module.Path] = true
		}
		p := &pkg{path: l.ImportPath, name: l.Name, dir: l.Dir, decls: map[string][]string{}}
		x := &pkg{path: l.ImportPath + "_test", dir: l.Dir, decls: map[string][]string{}}
		// Tests too (archgraph drops them unless --include-tests). IgnoredGoFiles are the files
		// for other platforms, cgo files (cgo is off) and files behind other build tags; all but
		// those needing `ignore` count: every platform's files at once.
		names := append(append(append([]string{}, l.GoFiles...), l.CgoFiles...), l.TestGoFiles...)
		names = append(names, l.IgnoredGoFiles...)
		for _, name := range names {
			f := parse(filepath.Join(l.Dir, name))
			if f == nil || ignored(f) {
				continue
			}
			if p.name == "" && !strings.HasSuffix(name, "_test.go") {
				p.name = f.Name.Name
			}
			switch f.Name.Name {
			case p.name:
				p.files = append(p.files, f)
			case p.name + "_test":
				x.files = append(x.files, f)
			}
		}
		for _, name := range l.XTestGoFiles {
			if f := parse(filepath.Join(l.Dir, name)); f != nil && !ignored(f) {
				x.files = append(x.files, f)
			}
		}
		p.testSupport = strings.HasSuffix(p.name, "testing") || strings.HasSuffix(p.name, "testutil")
		if len(p.files) > 0 {
			local[p.path], byPath[p.path] = p, p
		}
		if len(x.files) > 0 {
			xtests = append(xtests, x)
			byPath[x.path] = x
		}
	}

	var all []*pkg
	for _, p := range local {
		all = append(all, p)
	}
	all = append(all, xtests...)
	sort.Slice(all, func(i, j int) bool { return all[i].path < all[j].path })
	var files []fileOut
	edges := map[edge]bool{}
	unresolved := 0
	for _, p := range all {
		index(p)
		check(p)
		for _, f := range p.files {
			byFile[rel(f)] = p
		}
	}
	for _, p := range all {
		unresolved += references(p, edges)
	}
	// Who uses each file, for test support another package's production code relies on.
	usedBy := map[string][]string{}
	for e := range edges {
		usedBy[e.To] = append(usedBy[e.To], e.From)
	}
	usesTesting := map[string]bool{}
	for _, p := range all {
		for f := range testingUsers(p) {
			usesTesting[f] = true
		}
	}
	for _, p := range all {
		support := testSupport(p, usesTesting, usedBy)
		for _, f := range p.files {
			from := rel(f)
			files = append(files, fileOut{from, ast.IsGenerated(f), p.name == "main", support[from]})
		}
	}

	sort.Slice(files, func(i, j int) bool { return files[i].Path < files[j].Path })
	sorted := func(set map[edge]bool) []edge {
		out := []edge{}
		for e := range set {
			out = append(out, e)
		}
		sort.Slice(out, func(i, j int) bool {
			return out[i].From < out[j].From || out[i].From == out[j].From && out[i].To < out[j].To
		})
		return out
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", " ")
	if err := enc.Encode(map[string]any{
		"go_version": strings.TrimPrefix(runtime.Version(), "go"),
		"files":      files,
		"edges":      sorted(edges),
		"bound":      sorted(bound),
		"unresolved": unresolved,
		"modules":    rels,
	}); err != nil {
		fail(err)
	}
}

func fail(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}

// The packages of the module in `dir` read with go/build, for a module the go command rejects
// (an unparsable go.mod, a module graph it cannot load offline): every directory below it but
// those holding a go.mod of their own (scanned, skipped or excluded: another module either way,
// as for the go command), what the go command skips (vendor, testdata, names starting with `.` or
// `_`) and node_modules, each package's import path its module path plus its directory.
func buildList(dir string) []listed {
	modPath := ""
	if data, err := os.ReadFile(filepath.Join(dir, "go.mod")); err == nil {
		for _, line := range strings.Split(string(data), "\n") {
			line, _, _ = strings.Cut(line, "//")
			if rest, ok := strings.CutPrefix(strings.TrimSpace(line), "module"); ok {
				modPath = strings.Trim(strings.TrimSpace(rest), `"`)
				break
			}
		}
	}
	ctx := build.Default
	ctx.CgoEnabled = false
	var pkgs []listed
	_ = filepath.WalkDir(dir, func(path string, d fs.DirEntry, err error) error {
		if err != nil || !d.IsDir() {
			return nil
		}
		name := d.Name()
		if path != dir && (name == "vendor" || name == "testdata" || name == "node_modules" ||
			strings.HasPrefix(name, ".") || strings.HasPrefix(name, "_")) {
			return filepath.SkipDir
		}
		if _, err := os.Stat(filepath.Join(path, "go.mod")); err == nil && path != dir {
			return filepath.SkipDir
		}
		bp, _ := ctx.ImportDir(path, 0)
		if bp == nil || len(bp.GoFiles)+len(bp.IgnoredGoFiles)+len(bp.TestGoFiles)+len(bp.XTestGoFiles) == 0 {
			return nil
		}
		r, _ := filepath.Rel(dir, path)
		importPath := modPath
		if r != "." {
			importPath += "/" + filepath.ToSlash(r)
		}
		pkgs = append(pkgs, listed{
			ImportPath: importPath, Name: bp.Name, Dir: path, Module: &struct{ Path string }{modPath},
			GoFiles: bp.GoFiles, CgoFiles: bp.CgoFiles, IgnoredGoFiles: bp.IgnoredGoFiles,
			TestGoFiles: bp.TestGoFiles, XTestGoFiles: bp.XTestGoFiles,
		})
		return nil
	})
	return pkgs
}

// The module's packages. `-find` loads no dependency packages and -mod=readonly keeps a stale
// vendor/modules.txt from failing it, but the go command may still need the module graph (a
// module replacing others by directory), which fails offline when the module cache lacks it:
// the caller then falls back to buildList.
func list(gobin, dir string) ([]listed, error) {
	run := func(args ...string) ([]byte, error) {
		cmd := exec.Command(gobin, args...)
		cmd.Dir = dir
		cmd.Env = append(os.Environ(), append(safeEnv, "GOFLAGS=-mod=readonly", "PWD="+dir)...)
		var stderr bytes.Buffer
		cmd.Stderr = &stderr
		out, err := cmd.Output()
		if err != nil {
			return nil, fmt.Errorf("go %s in %s: %v\n%s", strings.Join(args, " "), dir, err,
				strings.TrimSpace(stderr.String()))
		}
		return out, nil
	}
	out, err := run("list", "-e", "-find", "-json", "./...")
	if err != nil {
		return nil, err
	}
	var pkgs []listed
	dec := json.NewDecoder(bytes.NewReader(out))
	for {
		var l listed
		if err := dec.Decode(&l); err == io.EOF {
			break
		} else if err != nil {
			return nil, err
		}
		pkgs = append(pkgs, l)
	}
	return pkgs, nil
}

func inside(dir string) bool {
	r, err := filepath.Rel(root, dir)
	if err != nil || r == ".." || strings.HasPrefix(r, ".."+string(filepath.Separator)) {
		return false
	}
	for _, seg := range strings.Split(filepath.ToSlash(r), "/") {
		if seg == "vendor" {
			return false
		}
	}
	return true
}

func parse(path string) *ast.File {
	f, _ := parser.ParseFile(fset, path, nil, parser.ParseComments|parser.SkipObjectResolution)
	if f == nil || f.Name == nil {
		return nil
	}
	return f
}

// Whether the file's build constraint requires the `ignore` tag (generators, examples).
func ignored(f *ast.File) bool {
	for _, group := range f.Comments {
		if group.Pos() >= f.Package {
			break
		}
		for _, c := range group.List {
			if !constraint.IsGoBuild(c.Text) && !constraint.IsPlusBuild(c.Text) {
				continue
			}
			if expr, err := constraint.Parse(c.Text); err == nil && requires(expr, "ignore") {
				return true
			}
		}
	}
	return false
}

// Whether `expr` is false for every setting of its other tags while `tag` is unset.
func requires(expr constraint.Expr, tag string) bool {
	var tags []string
	var walk func(constraint.Expr)
	walk = func(e constraint.Expr) {
		switch e := e.(type) {
		case *constraint.AndExpr:
			walk(e.X)
			walk(e.Y)
		case *constraint.OrExpr:
			walk(e.X)
			walk(e.Y)
		case *constraint.NotExpr:
			walk(e.X)
		case *constraint.TagExpr:
			if e.Tag != tag {
				tags = append(tags, e.Tag)
			}
		}
	}
	walk(expr)
	// ponytail: brute force over the other tags; a constraint with more than 12 of them is
	// taken as not requiring `tag`.
	if len(tags) > 12 {
		return false
	}
	for bits := 0; bits < 1<<len(tags); bits++ {
		set := map[string]bool{}
		for i, t := range tags {
			set[t] = bits&(1<<i) != 0
		}
		if expr.Eval(func(t string) bool { return set[t] }) {
			return false
		}
	}
	return true
}

// The non-test files of `p` using package testing for more than testing.Testing (a production
// check whether it runs under go test).
func testingUsers(p *pkg) map[string]bool {
	users := map[string]bool{}
	for _, f := range p.files {
		file := rel(f)
		if strings.HasSuffix(file, "_test.go") {
			continue
		}
		ast.Inspect(f, func(n ast.Node) bool {
			if id, ok := n.(*ast.Ident); ok {
				if obj := info.Uses[id]; obj != nil && obj.Pkg() != nil &&
					obj.Pkg().Path() == "testing" && obj.Name() != "Testing" {
					users[file] = true
				}
			}
			return !users[file]
		})
	}
	return users
}

// The files of `p` that are test support: all of them for a package named *testing or
// *testutil. Otherwise each non-test file using package testing (`usesTesting`) that no
// production file of another package uses (`usedBy`, a production file being a non-test file
// that uses no testing itself): a stub beside production code others use stays. A file with
// platform variants (the same names declared in other files) is test support only when every
// variant is; that is repeated until nothing changes, so chains of variants settle the same way
// on every run. The other files of the package stay.
func testSupport(p *pkg, usesTesting map[string]bool, usedBy map[string][]string) map[string]bool {
	support := map[string]bool{}
	for _, f := range p.files {
		file := rel(f)
		if p.testSupport {
			support[file] = true
			continue
		}
		if !usesTesting[file] {
			continue
		}
		used := false
		for _, from := range usedBy[file] {
			other := byFile[from]
			if !strings.HasSuffix(from, "_test.go") && !usesTesting[from] && other != p &&
				(other == nil || !other.testSupport) {
				used = true
			}
		}
		support[file] = !used
	}
	if p.testSupport {
		return support
	}
	for changed := true; changed; {
		changed = false
		for key, declFiles := range p.decls {
			if key == "init" || key == "_" || len(declFiles) < 2 {
				continue
			}
			some, all := false, true
			for _, f := range declFiles {
				if strings.HasSuffix(f, "_test.go") {
					continue
				}
				some = some || support[f]
				all = all && support[f]
			}
			if some && !all {
				for _, f := range declFiles {
					if support[f] {
						support[f], changed = false, true
					}
				}
			}
		}
	}
	return support
}

func rel(f *ast.File) string {
	r, _ := filepath.Rel(root, fset.Position(f.Package).Filename)
	return filepath.ToSlash(r)
}

func index(p *pkg) {
	for _, f := range p.files {
		file := rel(f)
		add := func(key string) {
			for _, have := range p.decls[key] {
				if have == file {
					return
				}
			}
			p.decls[key] = append(p.decls[key], file)
		}
		for _, d := range f.Decls {
			switch d := d.(type) {
			case *ast.FuncDecl:
				if d.Recv == nil || len(d.Recv.List) == 0 {
					add(d.Name.Name)
				} else if t := recvName(d.Recv.List[0].Type); t != "" {
					add(t + "." + d.Name.Name)
				}
			case *ast.GenDecl:
				for _, s := range d.Specs {
					switch s := s.(type) {
					case *ast.TypeSpec:
						add(s.Name.Name)
					case *ast.ValueSpec:
						for _, n := range s.Names {
							add(n.Name)
						}
					}
				}
			}
		}
	}
	for key, declFiles := range p.decls {
		// One name declared in several files of a package: platform variants under exclusive
		// build constraints (notify_linux.go, notify_windows.go), which move together or not at
		// all. Every file may declare init.
		if key != "init" && key != "_" {
			for _, other := range declFiles[1:] {
				bound[edge{declFiles[0], other}] = true
			}
		}
		methodFiles := declFiles
		typ, _, method := strings.Cut(key, ".")
		if !method {
			continue
		}
		for _, mf := range methodFiles {
			for _, tf := range p.decls[typ] {
				if mf != tf {
					bound[edge{mf, tf}] = true
				}
			}
		}
	}
}

func recvName(e ast.Expr) string {
	for {
		switch t := e.(type) {
		case *ast.StarExpr:
			e = t.X
		case *ast.ParenExpr:
			e = t.X
		case *ast.IndexExpr:
			e = t.X
		case *ast.IndexListExpr:
			e = t.X
		case *ast.Ident:
			return t.Name
		default:
			return ""
		}
	}
}

type imp struct{}

// Local packages are type-checked from source here; the standard library comes from
// go/importer; anything else (module dependencies) is not loaded and stays unresolved.
func (imp) Import(path string) (*types.Package, error) {
	if p := local[path]; p != nil {
		if p.busy {
			return nil, fmt.Errorf("import cycle through %s", path)
		}
		return check(p), nil
	}
	if first, _, _ := strings.Cut(path, "/"); !strings.Contains(first, ".") {
		return std.Import(path)
	}
	return nil, fmt.Errorf("%s is not loaded", path)
}

var info = &types.Info{Uses: map[*ast.Ident]types.Object{}, Selections: map[*ast.SelectorExpr]*types.Selection{}}

func check(p *pkg) *types.Package {
	if p.types != nil {
		return p.types
	}
	p.busy = true
	conf := types.Config{Importer: imp{}, Error: func(error) {}, FakeImportC: true}
	p.types, _ = conf.Check(p.path, fset, p.files, info)
	p.busy = false
	return p.types
}

// Adds an edge from each file of `p` to every local file declaring something it uses, and to the
// anchor file of a local package it imports blank (`import _`). Returns the number of imports
// that look local (under a scanned module path) but name no local package, plus the imports of
// local packages none of whose uses resolved.
func references(p *pkg, edges map[edge]bool) int {
	unresolved := 0
	for _, f := range p.files {
		from := rel(f)
		reached := map[*pkg]bool{}
		link := func(to string, q *pkg, obj types.Object) {
			reached[q] = true
			if to != from {
				edges[edge{from, to}] = true
				if q == p && !obj.Exported() {
					bound[edge{from, to}] = true
				}
			}
		}
		ast.Inspect(f, func(n ast.Node) bool {
			var obj types.Object
			switch n := n.(type) {
			case *ast.Ident:
				obj = info.Uses[n]
			case *ast.SelectorExpr:
				if s := info.Selections[n]; s != nil {
					obj = s.Obj()
				}
			}
			if obj == nil || obj.Pkg() == nil {
				return true
			}
			q := byPath[obj.Pkg().Path()]
			if q == nil {
				return true
			}
			if files := q.decls[key(obj)]; len(files) > 0 {
				for _, to := range files {
					link(to, q, obj)
				}
			} else if pos := obj.Pos(); pos.IsValid() {
				if r, err := filepath.Rel(root, fset.Position(pos).Filename); err == nil {
					link(filepath.ToSlash(r), q, obj)
				}
			}
			return true
		})
		for _, spec := range f.Imports {
			path := strings.Trim(spec.Path.Value, `"`)
			q := local[path]
			if q == nil {
				if underModule(path) {
					unresolved++
				}
				continue
			}
			switch {
			case reached[q] || q == p:
			case spec.Name != nil && spec.Name.Name == "_":
				// A blank import uses the package for its side effects: its anchor stands for it.
				reached[q] = true
				edges[edge{from, anchor(q)}] = true
			default:
				// Imported, but no use was resolved: they sit inside expressions whose type comes
				// from a module that is not loaded. Counted, not guessed.
				unresolved++
			}
		}
	}
	return unresolved
}

// The index key of a package-level object or method, "" for anything else.
func key(obj types.Object) string {
	if obj.Parent() != nil && obj.Parent() == obj.Pkg().Scope() {
		return obj.Name()
	}
	if fn, ok := obj.(*types.Func); ok {
		if recv := fn.Type().(*types.Signature).Recv(); recv != nil {
			t := recv.Type()
			if ptr, ok := t.(*types.Pointer); ok {
				t = ptr.Elem()
			}
			if named, ok := t.(*types.Named); ok {
				return named.Obj().Name() + "." + obj.Name()
			}
		}
	}
	return ""
}

// The file standing for a whole package: doc.go, else <dir>.go, else its first file; among
// files that are neither tests nor generated when it has any, so the edge survives excludes.
func anchor(p *pkg) string {
	var names, all []string
	for _, f := range p.files {
		all = append(all, rel(f))
		if !strings.HasSuffix(rel(f), "_test.go") && !ast.IsGenerated(f) {
			names = append(names, rel(f))
		}
	}
	if len(names) == 0 {
		names = all
	}
	sort.Strings(names)
	base := filepath.Base(p.dir)
	for _, want := range []string{"doc.go", base + ".go"} {
		for _, n := range names {
			if filepath.Base(n) == want {
				return n
			}
		}
	}
	return names[0]
}

func underModule(path string) bool {
	for m := range modules {
		if path == m || strings.HasPrefix(path, m+"/") {
			return true
		}
	}
	return false
}
