import com.sun.source.tree.*;
import com.sun.source.util.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import javax.lang.model.element.Element;
import javax.lang.model.element.PackageElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.type.TypeKind;
import javax.tools.*;

class ArchgraphJava {
    static String quote(String value) {
        StringBuilder out = new StringBuilder("\"");
        for (char c : value.toCharArray()) {
            if (c == '"' || c == '\\') out.append('\\').append(c);
            else if (c < 32) out.append(String.format("\\u%04x", (int) c));
            else out.append(c);
        }
        return out.append('"').toString();
    }

    public static void main(String[] args) throws Exception {
        Path root = Paths.get(args[0]);
        List<String> files = Arrays.asList(new String(System.in.readAllBytes(), StandardCharsets.UTF_8).split("\u0000"));
        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null) throw new IllegalStateException("a full JDK with jdk.compiler is required");
        DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
        TreeSet<String> edges = new TreeSet<>();
        List<String> imports = new ArrayList<>();
        Set<String> known = new HashSet<>(files);
        try (StandardJavaFileManager manager = compiler.getStandardFileManager(diagnostics, Locale.ROOT, StandardCharsets.UTF_8)) {
            List<Path> paths = new ArrayList<>();
            for (String file : files) paths.add(root.resolve(file));
            // No project classpath, implicit source discovery, annotation processors, or generation.
            JavacTask task = (JavacTask) compiler.getTask(null, manager, diagnostics,
                Arrays.asList("-proc:none", "-implicit:none", "--class-path", ".", "--source-path", ".", "-Xmaxerrs", "100000"),
                null, manager.getJavaFileObjectsFromPaths(paths));
            List<CompilationUnitTree> units = new ArrayList<>();
            for (CompilationUnitTree unit : task.parse()) units.add(unit);
            task.analyze();
            Trees trees = Trees.instance(task);
            for (CompilationUnitTree unit : units) {
                String from = root.relativize(Paths.get(unit.getSourceFile().toUri())).toString().replace('\\', '/');
                for (ImportTree item : unit.getImports()) {
                    Tree imported = item.getQualifiedIdentifier();
                    String specifier = imported.toString();
                    boolean wildcard = specifier.endsWith(".*");
                    Tree lookup = wildcard ? ((MemberSelectTree) imported).getExpression() : imported;
                    Element element = trees.getElement(TreePath.getPath(unit, lookup));
                    if (element == null && item.isStatic() && !wildcard && imported instanceof MemberSelectTree) {
                        MemberSelectTree member = (MemberSelectTree) imported;
                        Element owner = trees.getElement(TreePath.getPath(unit, member.getExpression()));
                        if (owner instanceof TypeElement) {
                            for (Element candidate : task.getElements().getAllMembers((TypeElement) owner)) {
                                if (candidate.getSimpleName().contentEquals(member.getIdentifier())) {
                                    element = candidate;
                                    break;
                                }
                            }
                        }
                    }
                    long position = trees.getSourcePositions().getStartPosition(unit, item);
                    long endPosition = trees.getSourcePositions().getEndPosition(unit, item);
                    boolean rejected = diagnostics.getDiagnostics().stream().anyMatch(d ->
                        d.getKind() == Diagnostic.Kind.ERROR && d.getSource() != null
                        && d.getSource().toUri().equals(unit.getSourceFile().toUri())
                        && d.getPosition() >= position && d.getPosition() < endPosition);
                    TreeSet<String> targets = new TreeSet<>();
                    String external = null;
                    // javac creates placeholder package elements even for missing wildcard packages.
                    if (!rejected && element != null && element.asType().getKind() != TypeKind.ERROR) {
                        if (element instanceof PackageElement) {
                            for (CompilationUnitTree candidate : units) {
                                if (candidate.getPackageName() != null
                                    && candidate.getPackageName().toString().equals(element.toString())) {
                                    targets.add(root.relativize(Paths.get(candidate.getSourceFile().toUri())).toString().replace('\\', '/'));
                                }
                            }
                        } else {
                            Element declarationElement = element;
                            TreePath declaration = trees.getPath(declarationElement);
                            while (declaration == null && declarationElement.getEnclosingElement() != null
                                && !(declarationElement.getEnclosingElement() instanceof PackageElement)) {
                                declarationElement = declarationElement.getEnclosingElement();
                                declaration = trees.getPath(declarationElement);
                            }
                            if (declaration != null) {
                                String target = root.relativize(Paths.get(declaration.getCompilationUnit().getSourceFile().toUri())).toString().replace('\\', '/');
                                if (known.contains(target)) targets.add(target);
                            }
                        }
                        if (targets.isEmpty()) {
                            Element owner = element;
                            while (owner != null && !(owner instanceof PackageElement)) owner = owner.getEnclosingElement();
                            if (owner != null) external = owner.toString();
                        }
                    }
                    long line = unit.getLineMap().getLineNumber(position);
                    List<String> quotedTargets = new ArrayList<>();
                    for (String target : targets) quotedTargets.add(quote(target));
                    imports.add("{\"from\":" + quote(from) + ",\"specifier\":" + quote(specifier)
                        + ",\"line\":" + line + ",\"kind\":\"runtime\",\"targets\":[" + String.join(",", quotedTargets) + "]"
                        + (external == null ? "" : ",\"external\":" + quote(external))
                        + (targets.isEmpty() && external == null ? ",\"unresolved\":\"missing\"" : "") + "}");
                }
                new TreePathScanner<Void, Void>() {
                    void reference() {
                        Element element = trees.getElement(getCurrentPath());
                        if (element == null || element instanceof PackageElement) return;
                        TreePath declaration = trees.getPath(element);
                        if (declaration == null) return;
                        String to = root.relativize(Paths.get(declaration.getCompilationUnit().getSourceFile().toUri())).toString().replace('\\', '/');
                        if (!from.equals(to) && known.contains(to)) {
                            edges.add("{\"from\":" + quote(from) + ",\"to\":" + quote(to) + "}");
                        }
                    }
                    @Override public Void visitIdentifier(IdentifierTree tree, Void unused) {
                        reference();
                        return super.visitIdentifier(tree, unused);
                    }
                    @Override public Void visitMemberSelect(MemberSelectTree tree, Void unused) {
                        reference();
                        return super.visitMemberSelect(tree, unused);
                    }
                    @Override public Void visitMemberReference(MemberReferenceTree tree, Void unused) {
                        reference();
                        return super.visitMemberReference(tree, unused);
                    }
                }.scan(unit, null);
            }
        }
        long errors = diagnostics.getDiagnostics().stream().filter(d -> d.getKind() == Diagnostic.Kind.ERROR).count();
        List<String> quoted = new ArrayList<>();
        for (String file : files) quoted.add(quote(file));
        System.out.println("{\"java_version\":" + quote(System.getProperty("java.version"))
            + ",\"files\":[" + String.join(",", quoted) + "],\"edges\":[" + String.join(",", edges)
            + "],\"imports\":[" + String.join(",", imports) + "],\"unresolved\":" + errors + "}");
    }
}
