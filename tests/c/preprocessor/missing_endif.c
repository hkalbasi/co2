//@ mode: c
//@ compile-fail

#define TRUE

     #ifdef TRUE
    //^^^^^ Unterminated conditional directive 
int main(void) {
    return 0;
}
