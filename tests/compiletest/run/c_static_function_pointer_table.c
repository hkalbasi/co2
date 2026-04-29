//@ mode: c

#include <stddef.h>

typedef int JSAtom;
typedef void JSClassFinalizer(void *rt, void *val);
typedef void JSClassGCMark(void *rt, void *val, void *mark);

typedef struct JSClassShortDef {
    JSAtom class_name;
    JSClassFinalizer *finalizer;
    JSClassGCMark *gc_mark;
} JSClassShortDef;

static void finalizer(void *rt, void *val) {}

void* func = (void*)&finalizer;

static const JSClassShortDef defs[] = {
    { 1, NULL, NULL },
    { 2, finalizer, NULL },
    { 3, NULL, func },
};

int main(void) {
    return defs[1].finalizer == finalizer && defs[0].gc_mark == NULL ? 0 : 1;
}
