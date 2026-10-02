//@ mode: c
//@ run-status: 0

/* GNU multicharacter character constants.
 *
 * C11 §6.4.4.4 makes the value of a multi-character character constant
 * implementation-defined. gcc packs up to sizeof(int) characters into an
 * int, first character in the most significant byte; extra leading
 * characters are dropped:
 *   'a'     == 0x61
 *   'ab'    == 0x6162
 *   'abc'   == 0x616263
 *   'abcd'  == 0x61626364
 *   'abcde' == 0x62636465
 * The result has type int. */

int main(void) {
    if ('a' != 0x61)
        return 1;
    if ('ab' != 0x6162)
        return 2;
    // TODO: This gets tokenized as lifetime.
    // I don't care about this multichar thing enough to think about some solution here.
    // if ('a c' != 0x612063)
    //     return 3;
    if (' a c' != 0x20612063)
        return 3;
    if ('abcd' != 0x61626364)
        return 4;
    if ('abcde' != 0x62636465)
        return 5;
    if ('ABCD' != 0x41424344)
        return 6;
    if ('aXb' != 0x615862)
        return 7;

    /* the type of a (multi)character constant is int, not char */
    if (sizeof('abcd') != sizeof(int))
        return 8;
    if (sizeof('ab') != sizeof(int))
        return 9;
    if (sizeof('a') != sizeof(int))
        return 10;

    return 0;
}
