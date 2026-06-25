//@ mode: c
//@ run-status: 0
//@ run-stdout: 1 2 4 10\n

#include <stddef.h>

typedef int compar(const void *, const void *);

void printf(const char*, ...);
void qsort(
    void *base,
    size_t nmemb,
    size_t size,
    compar cb
);
int compare_ints(const void *a, const void *b) {
    int x = *(const int *)a;
    int y = *(const int *)b;

    return x - y;
}

int main() {
    int arr[] = {4, 1, 10, 2};
    size_t n = sizeof(arr) / sizeof(arr[0]);

    compar *cb = compare_ints;

    qsort(arr, n, sizeof(int), cb);

    for (size_t i = 0; i < n; i++) {
        printf("%d ", arr[i]);
    }

    return 0;
}
