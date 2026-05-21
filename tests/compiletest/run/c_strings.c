//@ mode: c
//@ run-status: 0
//@ run-stdout: FILE: c_strings.out

#include <stdio.h>
#include <string.h>
#include <wchar.h>
#include <uchar.h>
#include <stdint.h>

static void separator(const char *name)
{
    printf("\n==================== %s ====================\n", name);
}

int main(void)
{
    separator("1. basic literals");

    puts("hello");
    puts("");
    puts(" ");
    puts("1234567890");

    separator("2. standard escapes");

    puts("newline:\nNEXT");
    puts("tab:\tX");
    puts("quote: \" ");
    puts("single quote: \' ");
    puts("backslash: \\");
    puts("question mark: \?");
    puts("alert:\a");
    puts("backspace:\bX");
    puts("formfeed:\fX");
    puts("carriage return:\rX");
    puts("vertical tab:\vX");

    separator("3. octal escapes");

    printf("\\0      = %d\n", "\0"[0]);
    printf("\\7      = %d\n", "\7"[0]);
    printf("\\12     = %d\n", "\12"[0]);
    printf("\\123    = %d\n", "\123"[0]);

    puts("A\\101B:");
    puts("A\101B");

    separator("4. hex escapes");

    printf("\\x41    = %d\n", "\x41"[0]);
    printf("\\x7f    = %d\n", "\x7f"[0]);

    puts("A\\x42C:");
    puts("A\x42C");
    //     ^^^^^ warning: hex escape sequence out of range; using low 8 bits

    {
        const char *s = "\x4142";
        //               ^^^^^^ warning: hex escape sequence out of range; using low 8 bits

        printf("sizeof(\"\\\\x4142\") = %zu\n", sizeof("\x4142"));
        //                                              ^^^^^^ warning: hex escape sequence out of range; using low 8 bits
        printf("first byte          = %u\n",
            (unsigned)(unsigned char)s[0]);
    }

    separator("5. adjacent concatenation");

    puts("hello " "world");

    puts("" "" "");

    puts(
        "abc"
        "def"
        "ghi"
    );

    separator("6. embedded NUL");

    {
        const char s[] = "abc\0def";

        printf("sizeof = %zu\n", sizeof(s));
        printf("strlen = %zu\n", strlen(s));

        printf("bytes   = ");

        for (size_t i = 0; i < sizeof(s); i++) {
            printf("%u ", (unsigned)(unsigned char)s[i]);
        }

        printf("\n");
    }

    separator("7. backslash-newline splicing");

    puts("hello \
world");

    separator("8. multiline adjacent literals");

    puts(
        "line1\n"
        "line2\n"
        "line3"
    );

    separator("9. UTF-8 source literals");

    puts("こんにちは");
    puts("😀");
    puts("Grüße");
    puts("Καλημέρα");
    puts("π");
    puts("日本");

    separator("10. universal character names");

    puts("\u03C0");
    puts("\u65E5");
    puts("\U0001F600");

    separator("11. UTF-prefixed literals");

    {
        const char *a = u8"hello";
        const char16_t *b = u"hello";
        const char32_t *c = U"hello";

        printf("u8 literal pointer size  = %zu\n", sizeof(*a));
        printf("u literal pointer size   = %zu\n", sizeof(*b));
        printf("U literal pointer size   = %zu\n", sizeof(*c));
    }

    separator("13. escape edge cases");

    puts("\\x");
    puts("\\u");
    puts("\\U");
    puts("\\8");
    puts("\\9");

    separator("14. sizeof tests");

    printf("sizeof(\"\")           = %zu\n", sizeof(""));
    printf("sizeof(\"a\")          = %zu\n", sizeof("a"));
    printf("sizeof(\"abc\")        = %zu\n", sizeof("abc"));
    printf("sizeof(\"abc\\0def\")  = %zu\n", sizeof("abc\0def"));

    separator("15. null terminator synthesis");

    {
        char a[] = "abc";
        char b[] = {'a', 'b', 'c'};

        printf("sizeof(a) = %zu\n", sizeof(a));
        printf("sizeof(b) = %zu\n", sizeof(b));

        puts(a);

        /*
            UB if uncommented:
            puts(b);
        */
    }

    separator("16. long literal");

    {
        const char *s =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        printf("strlen(long literal) = %zu\n", strlen(s));
    }

    separator("17. printf interactions");

    printf("%s\n", "hello");
    printf("100%%\n");
    printf("%c\n", '\n');

    separator("18. char vs string literals");

    printf("'A' as int      = %d\n", 'A');
    printf("sizeof('A')     = %zu\n", sizeof('A'));
    printf("sizeof(\"A\")    = %zu\n", sizeof("A"));

    separator("19. escaped newline inside literal");

    puts("abc\
def");

    puts("\
n");

    separator("20. comments between literals");

    puts("abc" /* comment */ "def");

    separator("21. octal vs hex greediness");

    puts("\x41G");
    puts("\101G");

    separator("22. literal bytes");

    {
        const unsigned char *s =
            (const unsigned char *)"é";

        printf("bytes of \"é\": ");

        for (size_t i = 0; s[i] != 0; i++) {
            printf("%02X ", s[i]);
        }

        printf("\n");
    }

    separator("23. giant mixed torture test");

    {
        const char *s =
            "ASCII\n"
            "\tTAB\n"
            "\x41\x42\x43\n"
            "\101\102\103\n"
            "UTF8: π 😀 日本\n"
            "QUOTE: \"\n"
            "SLASH: \\\n"
            "NUL:\0AFTER";

        fwrite(
            s,
            1,
            sizeof(
                "ASCII\n"
                "\tTAB\n"
                "\x41\x42\x43\n"
                "\101\102\103\n"
                "UTF8: π 😀 日本\n"
                "QUOTE: \"\n"
                "SLASH: \\\n"
                "NUL:\0AFTER"
            ) - 1,
            stdout
        );

        printf("\nstrlen=%zu\n", strlen(s));
    }

    separator("24. pointer equality optimization");

    {
        const char *a = "hello";
        const char *b = "hello";

        printf("a == b : %d\n", a == b);
    }

    separator("25. escape parsing boundaries");

    {
        const char *a = "\1234";
        const char *b = "\x414243";
        //               ^^^^^^^^ warning: hex escape sequence out of range; using low 8 bits
        const char *c = "\12""3";

        printf("a bytes: ");
        for (size_t i = 0; a[i]; i++) {
            printf("%02X ", (unsigned char)a[i]);
        }
        printf("\n");

        printf("b first byte: %02X\n",
            (unsigned char)b[0]);

        printf("c bytes: ");
        for (size_t i = 0; c[i]; i++) {
            printf("%02X ", (unsigned char)c[i]);
        }
        printf("\n");
    }

    separator("26. explicit array contents");

    {
        const char s[] = "A\0B\0C";

        printf("sizeof = %zu\n", sizeof(s));

        for (size_t i = 0; i < sizeof(s); i++) {
            printf("[%zu]=%u ", i,
                (unsigned)(unsigned char)s[i]);
        }

        printf("\n");
    }

    separator("27. UTF literal sizes");

    {
        printf("sizeof(u\"A\")  = %zu\n", sizeof(u"A"));
        printf("sizeof(U\"A\")  = %zu\n", sizeof(U"A"));
        printf("sizeof(L\"A\")  = %zu\n", sizeof(L"A"));
        printf("sizeof(u8\"A\") = %zu\n", sizeof(u8"A"));
    }

    separator("28. concatenation corner cases");

    {
        const char *s =
            ""
            "a"
            ""
            "b"
            ""
            "c";

        puts(s);
    }

    separator("29. hex escape swallowing");

    {
        const char *a = "\x41""2";
        const char *b = "\x412";
        //               ^^^^^ warning: hex escape sequence out of range; using low 8 bits

        printf("a bytes: ");
        for (size_t i = 0; a[i]; i++) {
            printf("%02X ", (unsigned char)a[i]);
        }
        printf("\n");

        printf("b first byte: %02X\n",
            (unsigned char)b[0]);
    }

    separator("30. done");

    return 0;
}
