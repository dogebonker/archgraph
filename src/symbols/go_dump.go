// Source-only symbol collection. The helper is compiled outside the scanned tree; repository
// packages are parsed and type-checked, never compiled or executed.
package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/importer"
	"go/parser"
	"go/token"
	"go/types"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

type target struct {
	File string `json:"file"`
	Name string `json:"name"`
}
type export struct {
	File   string `json:"file"`
	Name   string `json:"name"`
	Target target `json:"target"`
}
type use struct {
	From     string `json:"from"`
	To       string `json:"to"`
	Name     string `json:"name"`
	Target   target `json:"target"`
	Wildcard bool   `json:"wildcard"`
}
type result struct {
	Complete    bool     `json:"complete"`
	Exports     []export `json:"exports"`
	Imports     []use    `json:"imports"`
	Limitations []string `json:"limitations"`
}
type pkg struct {
	path    string
	files   []*ast.File
	checked *types.Package
	busy    bool
}

var (
	root     string
	fset     = token.NewFileSet()
	packages = map[string]*pkg{}
	info     = &types.Info{Defs: map[*ast.Ident]types.Object{}, Uses: map[*ast.Ident]types.Object{}, Selections: map[*ast.SelectorExpr]*types.Selection{}, Types: map[ast.Expr]types.TypeAndValue{}}
	standard = importer.ForCompiler(fset, "source", nil)
	gaps     = map[string]bool{}
	paths    = map[string]string{}
	fields   = map[token.Pos]string{}
)

type imports struct{}

func (imports) Import(path string) (*types.Package, error) {
	if p := packages[path]; p != nil {
		if p.busy {
			return nil, fmt.Errorf("import cycle through %s", path)
		}
		return check(p), nil
	}
	first, _, _ := strings.Cut(path, "/")
	if !strings.Contains(first, ".") {
		return standard.Import(path)
	}
	return nil, fmt.Errorf("external package %s is not loaded", path)
}
func check(p *pkg) *types.Package {
	if p.checked != nil {
		return p.checked
	}
	p.busy = true
	conf := types.Config{Importer: imports{}, FakeImportC: true, Error: func(err error) {
		gaps[fmt.Sprintf("Go type checking %s was incomplete: %s", p.path, err)] = true
	}}
	p.checked, _ = conf.Check(p.path, fset, p.files, info)
	p.busy = false
	return p.checked
}
func packagePath(dir string) (string, error) {
	for current := dir; ; current = filepath.Dir(current) {
		if data, err := os.ReadFile(filepath.Join(current, "go.mod")); err == nil {
			for _, line := range strings.Split(string(data), "\n") {
				fields := strings.Fields(line)
				if len(fields) >= 2 && fields[0] == "module" {
					rel, err := filepath.Rel(current, dir)
					name := strings.Trim(fields[1], `"`)
					if rel != "." {
						name += "/" + filepath.ToSlash(rel)
					}
					return name, err
				}
			}
		}
		if current == root || filepath.Dir(current) == current {
			break
		}
	}
	return "", fmt.Errorf("no module declaration for %s", dir)
}

// Field positions survive type aliases and generic instantiation, preserving declaration identity.
func key(obj types.Object) string {
	if obj == nil || obj.Pkg() == nil || !obj.Exported() {
		return ""
	}
	if field, ok := obj.(*types.Var); ok && field.IsField() {
		return fields[field.Pos()]
	}
	if obj.Parent() == obj.Pkg().Scope() {
		return obj.Name()
	}
	if fn, ok := obj.(*types.Func); ok {
		sig, ok := fn.Type().(*types.Signature)
		if !ok || sig.Recv() == nil {
			return ""
		}
		typ := sig.Recv().Type()
		if ptr, ok := typ.(*types.Pointer); ok {
			typ = ptr.Elem()
		}
		if named, ok := typ.(*types.Named); ok {
			return named.Obj().Name() + "." + obj.Name()
		}
	}
	return ""
}
func canonical(obj types.Object) (target, bool) {
	name := key(obj)
	if name == "" {
		return target{}, false
	}
	filename := fset.Position(obj.Pos()).Filename
	file, ok := paths[filename]
	return target{file, name}, ok
}
func fail(err error) { fmt.Fprintln(os.Stderr, err); os.Exit(1) }
func main() {
	if len(os.Args) != 2 {
		fail(fmt.Errorf("usage: go_dump <root>"))
	}
	root = os.Args[1]
	var files []string
	if err := json.NewDecoder(os.Stdin).Decode(&files); err != nil {
		fail(err)
	}
	sort.Strings(files)
	for _, file := range files {
		absolute := filepath.Join(root, filepath.FromSlash(file))
		f, err := parser.ParseFile(fset, absolute, nil, parser.SkipObjectResolution)
		if err != nil {
			gaps[fmt.Sprintf("Go parsing %s was incomplete: %s", file, err)] = true
		}
		if f == nil {
			continue
		}
		path, err := packagePath(filepath.Dir(absolute))
		if err != nil {
			gaps[err.Error()] = true
			continue
		}
		if strings.HasSuffix(f.Name.Name, "_test") {
			path += "_test"
		}
		p := packages[path]
		if p == nil {
			p = &pkg{path: path}
			packages[path] = p
		}
		p.files = append(p.files, f)
		paths[absolute] = file
	}
	var names []string
	for name := range packages {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		check(packages[name])
	}
	// Named struct fields retain their declaring type through aliases and promoted selectors.
	for _, name := range names {
		for _, file := range packages[name].files {
			ast.Inspect(file, func(node ast.Node) bool {
				spec, ok := node.(*ast.TypeSpec)
				if !ok {
					return true
				}
				owner := info.Defs[spec.Name]
				if owner == nil || owner.Pkg() == nil || owner.Parent() != owner.Pkg().Scope() {
					return true
				}
				structure, ok := spec.Type.(*ast.StructType)
				if !ok {
					return true
				}
				for _, field := range structure.Fields.List {
					for _, name := range field.Names {
						if name.IsExported() {
							fields[name.Pos()] = spec.Name.Name + "." + name.Name
						}
					}
					if len(field.Names) == 0 {
						// Embedded fields have no identifier in Defs, but their type object retains the position.
						obj := info.Defs[spec.Name]
						if typ, ok := obj.(*types.TypeName); ok {
							if st, ok := typ.Type().Underlying().(*types.Struct); ok {
								for i := 0; i < st.NumFields(); i++ {
									member := st.Field(i)
									if member.Exported() && member.Pos() >= field.Pos() && member.Pos() <= field.End() {
										fields[member.Pos()] = spec.Name.Name + "." + member.Name()
									}
								}
							}
						}
					}
				}
				return true
			})
		}
	}
	out := result{Exports: []export{}, Imports: []use{}, Limitations: []string{}}
	exportSet := map[target]bool{}
	for _, obj := range info.Defs {
		if field, ok := obj.(*types.Var); ok && field.IsField() && field.Exported() && fields[field.Pos()] == "" {
			gaps["Exported fields in anonymous structs have no stable type-qualified name; Go field coverage is incomplete."] = true
		}
		if t, ok := canonical(obj); ok && !exportSet[t] {
			exportSet[t] = true
			out.Exports = append(out.Exports, export{t.File, t.Name, t})
		}
	}
	for pos, name := range fields {
		if file, ok := paths[fset.Position(pos).Filename]; ok {
			t := target{file, name}
			if !exportSet[t] {
				exportSet[t] = true
				out.Exports = append(out.Exports, export{file, name, t})
			}
		}
	}
	// Include instantiated concrete types observed inside factories as well as declarations.
	concrete := map[*types.Named]bool{}
	addConcrete := func(typ types.Type) {
		if pointer, ok := typ.(*types.Pointer); ok {
			typ = pointer.Elem()
		}
		if named, ok := typ.(*types.Named); ok && named.Obj().Pkg() != nil && packages[named.Obj().Pkg().Path()] != nil {
			if _, isInterface := named.Underlying().(*types.Interface); !isInterface {
				concrete[named] = true
			}
		}
	}
	for _, obj := range info.Defs {
		if obj != nil {
			addConcrete(obj.Type())
		}
	}
	for _, value := range info.Types {
		addConcrete(value.Type)
	}
	seen := map[use]bool{}
	conservativeMethods := false
	for _, name := range names {
		p := packages[name]
		for _, f := range p.files {
			from := paths[fset.Position(f.Package).Filename]
			add := func(obj types.Object) {
				if obj == nil || obj.Pkg() == nil || obj.Pkg().Path() == p.path {
					return
				}
				if field, ok := obj.(*types.Var); ok && field.IsField() && field.Exported() && fields[field.Pos()] == "" {
					if _, local := paths[fset.Position(field.Pos()).Filename]; local {
						gaps["Exported fields in anonymous structs have no stable type-qualified name; Go field coverage is incomplete."] = true
					}
				}
				if t, ok := canonical(obj); ok {
					u := use{From: from, To: t.File, Name: t.Name, Target: t}
					if !seen[u] {
						seen[u] = true
						out.Imports = append(out.Imports, u)
					}
				}
			}
			interfacesSeen := map[*types.Interface]bool{}
			ast.Inspect(f, func(n ast.Node) bool {
				// Interface conversion can hide the eventual receiver: retain all exported methods of a
				// local named type observed by another package, including factory return values.
				if expression, ok := n.(ast.Expr); ok {
					typ := info.Types[expression].Type
					if pointer, ok := typ.(*types.Pointer); ok {
						typ = pointer.Elem()
					}
					if typ != nil {
						if contract, ok := typ.Underlying().(*types.Interface); ok && !interfacesSeen[contract] {
							interfacesSeen[contract] = true
							contract.Complete()
							// Named, anonymous and standard-library interfaces all hide concrete factory results.
							// Retain contract-required methods of compatible scanned types, including private ones.
							for candidate := range concrete {
								receiver := types.NewPointer(candidate)
								if !types.Implements(candidate, contract) && !types.Implements(receiver, contract) {
									continue
								}
								for i := 0; i < contract.NumMethods(); i++ {
									required := contract.Method(i)
									method, _, _ := types.LookupFieldOrMethod(receiver, true, required.Pkg(), required.Name())
									if method != nil && method.Exported() {
										add(method)
										conservativeMethods = true
									}
								}
							}
						}
					}
					if named, ok := typ.(*types.Named); ok && named.Obj().Pkg() != nil && named.Obj().Pkg().Path() != p.path && packages[named.Obj().Pkg().Path()] != nil {
						methods := types.NewMethodSet(types.NewPointer(named))
						for i := 0; i < methods.Len(); i++ {
							method := methods.At(i).Obj()
							if method.Exported() {
								add(method)
								conservativeMethods = true
							}
						}
					}
				}
				switch n := n.(type) {
				case *ast.Ident:
					add(info.Uses[n])
				case *ast.SelectorExpr:
					if s := info.Selections[n]; s != nil {
						add(s.Obj())
					}
				}
				return true
			})
		}
	}
	sort.Slice(out.Exports, func(i, j int) bool {
		a, b := out.Exports[i], out.Exports[j]
		return a.File < b.File || a.File == b.File && a.Name < b.Name
	})
	sort.Slice(out.Imports, func(i, j int) bool {
		a, b := out.Imports[i], out.Imports[j]
		if a.From != b.From {
			return a.From < b.From
		}
		if a.To != b.To {
			return a.To < b.To
		}
		return a.Name < b.Name
	})
	for gap := range gaps {
		out.Limitations = append(out.Limitations, gap)
	}
	out.Complete = len(gaps) == 0
	if conservativeMethods {
		out.Limitations = append(out.Limitations, "Go method usage is conservative: all exported methods of a scanned named type observed by another package count as used, including interface dispatch and factory returns. Required methods of compatible scanned implementations of a consumed public interface also count as used.")
	}
	sort.Strings(out.Limitations)
	if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
		fail(err)
	}
}
