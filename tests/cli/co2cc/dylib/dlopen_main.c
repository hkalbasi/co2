#include <dlfcn.h>
#include <stdio.h>

typedef int (*add42_fn)(int);

int main(void) {
    void *handle = dlopen("./libco2cc_dylib.so", RTLD_NOW);
    if (handle == NULL) {
        fprintf(stderr, "dlopen failed: %s\n", dlerror());
        return 2;
    }

    dlerror();
    add42_fn add42 = (add42_fn)dlsym(handle, "add42");
    const char *error = dlerror();
    if (error != NULL) {
        fprintf(stderr, "dlsym failed: %s\n", error);
        dlclose(handle);
        return 3;
    }

    int value = add42(8);
    if (dlclose(handle) != 0) {
        fprintf(stderr, "dlclose failed: %s\n", dlerror());
        return 4;
    }

    printf("%d\n", value);
    return value == 50 ? 0 : 5;
}
