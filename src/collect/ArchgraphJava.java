import com.sun.source.tree.*;
import com.sun.source.util.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import javax.lang.model.element.Element;
import javax.lang.model.element.PackageElement;
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
            + "],\"unresolved\":" + errors + "}");
    }
}
