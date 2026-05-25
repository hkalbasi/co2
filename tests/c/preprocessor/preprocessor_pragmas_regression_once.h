#pragma once

static int once_counter = 0;

static int once_helper(void) {
    return ++once_counter;
}
