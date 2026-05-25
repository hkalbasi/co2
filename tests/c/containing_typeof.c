//@ mode: c
//@ run-status: 0

typedef struct JSContext JSContext;
typedef int JSValue;

static int js_operator_typeof(JSContext *ctx, JSValue op1)
{
    return js_operator_typeof(0, 0) ? 0 : 1;
}

static int js_operator__extension__(JSContext *ctx, JSValue op1)
{
    return js_operator__extension__(0, 0) ? 0 : 1;
}

static int js_operator__inline__(JSContext *ctx, JSValue op1)
{
    return js_operator__inline__(0, 0) ? 0 : 1;
}

static int js_operator_Noreturn(JSContext *ctx, JSValue op1)
{
    return js_operator_Noreturn(0, 0) ? 0 : 1;
}

static int js_operator__attribute__(JSContext *ctx, JSValue op1)
{
    return js_operator__attribute__(0, 0) ? 0 : 1;
}

static int js_operator(JSContext *ctx, JSValue op1)
{
    return js_operator(0, 0) ? 0 : 1;
}

int main(void)
{
    return 0;
}
