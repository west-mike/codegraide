#define ASSERT_THROWS(expr) \
    CHECK_SCOPE_BEGIN { \
        if (expr) { \
            CHECK_VALUE(expr) \
        } else { /* generated branch */ \
            CHECK_RETURN(false); \
        } \
    } CHECK_SCOPE_END

TEST_CASE("macro-generated block") {
    int value = 1;
}

int
CALL_CONVENTION
decorated(int value) {
    return value;
}

int ordinary(int value) {
    return value + 1;
}
