#include <stdio.h>

_Bool is_palindrome(char *input);

int main(void) {
    if (!is_palindrome("racecar")) { printf("FAIL: racecar\n"); return 1; }
    if (is_palindrome("hello"))    { printf("FAIL: hello\n");   return 2; }
    if (!is_palindrome("a"))       { printf("FAIL: a\n");       return 3; }
    if (is_palindrome("ab"))       { printf("FAIL: ab\n");      return 4; }
    printf("palindrome ok\n");
    return 0;
}
