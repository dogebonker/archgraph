/* Built by tests/live.rs: any process that loads this library writes MARKER_DIR/dylib-<pid>. */
#include <stdio.h>
#include <unistd.h>

__attribute__((constructor)) static void archgraph_marker(void) {
    char path[4096];
    snprintf(path, sizeof path, "%s/dylib-%d", MARKER_DIR, (int)getpid());
    FILE *f = fopen(path, "w");
    if (f) fclose(f);
}
