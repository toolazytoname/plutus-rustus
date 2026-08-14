/* SIMD hash160 for the collider hot loop, x86_64 with SHA-NI.
 *
 * hash160(pubkey) = RIPEMD-160(SHA-256(pubkey)). SHA-256 uses Intel SHA
 * extensions (public-domain implementation by Jeffrey Walton / Intel / Sean
 * Gulley). RIPEMD-160 is 4-wide SSE2, matching the aarch64 NEON path.
 *
 * Callers must CPUID-check SHA-NI + SSE4.1 before invoking these functions.
 */
#if defined(__x86_64__)

#include <stdint.h>
#include <string.h>
#include <x86intrin.h>

/* Process `length` bytes of already-padded SHA-256 blocks. `state` is 8
 * uint32 big-endian SHA-256 words. Public domain: Jeffrey Walton / Intel. */
static void sha256_process_x86(uint32_t state[8], const uint8_t data[], uint32_t length)
{
    __m128i STATE0, STATE1;
    __m128i MSG, TMP;
    __m128i MSG0, MSG1, MSG2, MSG3;
    __m128i ABEF_SAVE, CDGH_SAVE;
    const __m128i MASK = _mm_set_epi64x(0x0c0d0e0f08090a0bULL, 0x0405060700010203ULL);

    TMP = _mm_loadu_si128((const __m128i*) &state[0]);
    STATE1 = _mm_loadu_si128((const __m128i*) &state[4]);

    TMP = _mm_shuffle_epi32(TMP, 0xB1);
    STATE1 = _mm_shuffle_epi32(STATE1, 0x1B);
    STATE0 = _mm_alignr_epi8(TMP, STATE1, 8);
    STATE1 = _mm_blend_epi16(STATE1, TMP, 0xF0);

    while (length >= 64)
    {
        ABEF_SAVE = STATE0;
        CDGH_SAVE = STATE1;

        MSG = _mm_loadu_si128((const __m128i*) (data+0));
        MSG0 = _mm_shuffle_epi8(MSG, MASK);
        MSG = _mm_add_epi32(MSG0, _mm_set_epi64x(0xE9B5DBA5B5C0FBCFULL, 0x71374491428A2F98ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);

        MSG1 = _mm_loadu_si128((const __m128i*) (data+16));
        MSG1 = _mm_shuffle_epi8(MSG1, MASK);
        MSG = _mm_add_epi32(MSG1, _mm_set_epi64x(0xAB1C5ED5923F82A4ULL, 0x59F111F13956C25BULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG0 = _mm_sha256msg1_epu32(MSG0, MSG1);

        MSG2 = _mm_loadu_si128((const __m128i*) (data+32));
        MSG2 = _mm_shuffle_epi8(MSG2, MASK);
        MSG = _mm_add_epi32(MSG2, _mm_set_epi64x(0x550C7DC3243185BEULL, 0x12835B01D807AA98ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG1 = _mm_sha256msg1_epu32(MSG1, MSG2);

        MSG3 = _mm_loadu_si128((const __m128i*) (data+48));
        MSG3 = _mm_shuffle_epi8(MSG3, MASK);
        MSG = _mm_add_epi32(MSG3, _mm_set_epi64x(0xC19BF1749BDC06A7ULL, 0x80DEB1FE72BE5D74ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG3, MSG2, 4);
        MSG0 = _mm_add_epi32(MSG0, TMP);
        MSG0 = _mm_sha256msg2_epu32(MSG0, MSG3);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG2 = _mm_sha256msg1_epu32(MSG2, MSG3);

        MSG = _mm_add_epi32(MSG0, _mm_set_epi64x(0x240CA1CC0FC19DC6ULL, 0xEFBE4786E49B69C1ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG0, MSG3, 4);
        MSG1 = _mm_add_epi32(MSG1, TMP);
        MSG1 = _mm_sha256msg2_epu32(MSG1, MSG0);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG3 = _mm_sha256msg1_epu32(MSG3, MSG0);

        MSG = _mm_add_epi32(MSG1, _mm_set_epi64x(0x76F988DA5CB0A9DCULL, 0x4A7484AA2DE92C6FULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG1, MSG0, 4);
        MSG2 = _mm_add_epi32(MSG2, TMP);
        MSG2 = _mm_sha256msg2_epu32(MSG2, MSG1);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG0 = _mm_sha256msg1_epu32(MSG0, MSG1);

        MSG = _mm_add_epi32(MSG2, _mm_set_epi64x(0xBF597FC7B00327C8ULL, 0xA831C66D983E5152ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG2, MSG1, 4);
        MSG3 = _mm_add_epi32(MSG3, TMP);
        MSG3 = _mm_sha256msg2_epu32(MSG3, MSG2);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG1 = _mm_sha256msg1_epu32(MSG1, MSG2);

        MSG = _mm_add_epi32(MSG3, _mm_set_epi64x(0x1429296706CA6351ULL, 0xD5A79147C6E00BF3ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG3, MSG2, 4);
        MSG0 = _mm_add_epi32(MSG0, TMP);
        MSG0 = _mm_sha256msg2_epu32(MSG0, MSG3);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG2 = _mm_sha256msg1_epu32(MSG2, MSG3);

        MSG = _mm_add_epi32(MSG0, _mm_set_epi64x(0x53380D134D2C6DFCULL, 0x2E1B213827B70A85ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG0, MSG3, 4);
        MSG1 = _mm_add_epi32(MSG1, TMP);
        MSG1 = _mm_sha256msg2_epu32(MSG1, MSG0);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG3 = _mm_sha256msg1_epu32(MSG3, MSG0);

        MSG = _mm_add_epi32(MSG1, _mm_set_epi64x(0x92722C8581C2C92EULL, 0x766A0ABB650A7354ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG1, MSG0, 4);
        MSG2 = _mm_add_epi32(MSG2, TMP);
        MSG2 = _mm_sha256msg2_epu32(MSG2, MSG1);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG0 = _mm_sha256msg1_epu32(MSG0, MSG1);

        MSG = _mm_add_epi32(MSG2, _mm_set_epi64x(0xC76C51A3C24B8B70ULL, 0xA81A664BA2BFE8A1ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG2, MSG1, 4);
        MSG3 = _mm_add_epi32(MSG3, TMP);
        MSG3 = _mm_sha256msg2_epu32(MSG3, MSG2);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG1 = _mm_sha256msg1_epu32(MSG1, MSG2);

        MSG = _mm_add_epi32(MSG3, _mm_set_epi64x(0x106AA070F40E3585ULL, 0xD6990624D192E819ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG3, MSG2, 4);
        MSG0 = _mm_add_epi32(MSG0, TMP);
        MSG0 = _mm_sha256msg2_epu32(MSG0, MSG3);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG2 = _mm_sha256msg1_epu32(MSG2, MSG3);

        MSG = _mm_add_epi32(MSG0, _mm_set_epi64x(0x34B0BCB52748774CULL, 0x1E376C0819A4C116ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG0, MSG3, 4);
        MSG1 = _mm_add_epi32(MSG1, TMP);
        MSG1 = _mm_sha256msg2_epu32(MSG1, MSG0);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);
        MSG3 = _mm_sha256msg1_epu32(MSG3, MSG0);

        MSG = _mm_add_epi32(MSG1, _mm_set_epi64x(0x682E6FF35B9CCA4FULL, 0x4ED8AA4A391C0CB3ULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG1, MSG0, 4);
        MSG2 = _mm_add_epi32(MSG2, TMP);
        MSG2 = _mm_sha256msg2_epu32(MSG2, MSG1);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);

        MSG = _mm_add_epi32(MSG2, _mm_set_epi64x(0x8CC7020884C87814ULL, 0x78A5636F748F82EEULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        TMP = _mm_alignr_epi8(MSG2, MSG1, 4);
        MSG3 = _mm_add_epi32(MSG3, TMP);
        MSG3 = _mm_sha256msg2_epu32(MSG3, MSG2);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);

        MSG = _mm_add_epi32(MSG3, _mm_set_epi64x(0xC67178F2BEF9A3F7ULL, 0xA4506CEB90BEFFFAULL));
        STATE1 = _mm_sha256rnds2_epu32(STATE1, STATE0, MSG);
        MSG = _mm_shuffle_epi32(MSG, 0x0E);
        STATE0 = _mm_sha256rnds2_epu32(STATE0, STATE1, MSG);

        STATE0 = _mm_add_epi32(STATE0, ABEF_SAVE);
        STATE1 = _mm_add_epi32(STATE1, CDGH_SAVE);

        data += 64;
        length -= 64;
    }

    TMP = _mm_shuffle_epi32(STATE0, 0x1B);
    STATE1 = _mm_shuffle_epi32(STATE1, 0xB1);
    STATE0 = _mm_blend_epi16(TMP, STATE1, 0xF0);
    STATE1 = _mm_alignr_epi8(STATE1, TMP, 8);

    _mm_storeu_si128((__m128i*) &state[0], STATE0);
    _mm_storeu_si128((__m128i*) &state[4], STATE1);
}

static const uint32_t SHA_IV[8] = {
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19
};

static void sha256_store_be(const uint32_t state[8], uint8_t out[32]) {
    for (int i = 0; i < 8; i++) {
        out[i * 4 + 0] = (uint8_t)(state[i] >> 24);
        out[i * 4 + 1] = (uint8_t)(state[i] >> 16);
        out[i * 4 + 2] = (uint8_t)(state[i] >> 8);
        out[i * 4 + 3] = (uint8_t)(state[i] >> 0);
    }
}

static void sha256_33(const uint8_t *msg, uint8_t out[32]) {
    uint8_t block[64];
    memcpy(block, msg, 33);
    block[33] = 0x80;
    memset(block + 34, 0, 64 - 34);
    block[62] = 0x01;
    block[63] = 0x08;
    uint32_t state[8];
    memcpy(state, SHA_IV, sizeof(SHA_IV));
    sha256_process_x86(state, block, 64);
    sha256_store_be(state, out);
}

static void sha256_65(const uint8_t *msg, uint8_t out[32]) {
    uint8_t blocks[128];
    memcpy(blocks, msg, 64);
    memset(blocks + 64, 0, 64);
    blocks[64] = msg[64];
    blocks[65] = 0x80;
    blocks[126] = 0x02;
    blocks[127] = 0x08;
    uint32_t state[8];
    memcpy(state, SHA_IV, sizeof(SHA_IV));
    sha256_process_x86(state, blocks, 128);
    sha256_store_be(state, out);
}

/* ---- 4-way SSE2 RIPEMD-160 for four fixed 32-byte inputs ---- */

static const uint8_t RL[80] = {
    0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,
    7,4,13,1,10,6,15,3,12,0,9,5,2,14,11,8,
    3,10,14,4,9,15,8,1,2,7,0,6,13,11,5,12,
    1,9,11,10,0,8,12,4,13,3,7,15,14,5,6,2,
    4,0,5,9,7,12,2,10,14,1,3,8,11,6,15,13};
static const uint8_t RR[80] = {
    5,14,7,0,9,2,11,4,13,6,15,8,1,10,3,12,
    6,11,3,7,0,13,5,10,14,15,8,12,4,9,1,2,
    15,5,1,3,7,14,6,9,11,8,12,2,10,0,4,13,
    8,6,4,1,3,11,15,0,5,12,2,13,9,7,10,14,
    12,15,10,4,1,5,8,7,6,2,13,14,0,3,9,11};
static const uint8_t SL[80] = {
    11,14,15,12,5,8,7,9,11,13,14,15,6,7,9,8,
    7,6,8,13,11,9,7,15,7,12,15,9,11,7,13,12,
    11,13,6,7,14,9,13,15,14,8,13,6,5,12,7,5,
    11,12,14,15,14,15,9,8,9,14,5,6,8,6,5,12,
    9,15,5,11,6,8,13,12,5,12,13,14,11,8,5,6};
static const uint8_t SR[80] = {
    8,9,9,11,13,15,15,5,7,7,8,11,14,14,12,6,
    9,13,15,7,12,8,9,11,7,7,12,7,6,15,13,11,
    9,7,15,11,8,6,6,14,12,13,5,14,13,13,7,5,
    15,5,8,11,14,14,6,14,6,9,12,9,12,5,15,8,
    8,5,12,9,12,5,14,6,8,13,6,5,15,13,11,11};
static const uint32_t KL[5] = {0x00000000,0x5A827999,0x6ED9EBA1,0x8F1BBCDC,0xA953FD4E};
static const uint32_t KR[5] = {0x50A28BE6,0x5C4DD124,0x6D703EF3,0x7A6D76E9,0x00000000};

static inline __m128i rotl_epi32(__m128i x, int n) {
    return _mm_or_si128(_mm_slli_epi32(x, n), _mm_srli_epi32(x, 32 - n));
}

static inline __m128i fr(int round, __m128i x, __m128i y, __m128i z) {
    switch (round) {
        case 0: return _mm_xor_si128(_mm_xor_si128(x, y), z);
        case 1: return _mm_or_si128(_mm_and_si128(x, y), _mm_andnot_si128(x, z));
        case 2: return _mm_xor_si128(_mm_or_si128(x, _mm_xor_si128(y, _mm_set1_epi32(-1))), z);
        case 3: return _mm_or_si128(_mm_and_si128(x, z), _mm_andnot_si128(z, y));
        default: return _mm_xor_si128(x, _mm_or_si128(y, _mm_xor_si128(z, _mm_set1_epi32(-1))));
    }
}

static void ripemd160_x4(const uint8_t *in, uint8_t *out) {
    __m128i X[16];
    for (int j = 0; j < 8; j++) {
        uint32_t t[4];
        for (int l = 0; l < 4; l++) memcpy(&t[l], in + l * 32 + j * 4, 4);
        X[j] = _mm_loadu_si128((const __m128i *)t);
    }
    X[8] = _mm_set1_epi32(0x00000080);
    for (int j = 9; j < 16; j++) X[j] = _mm_setzero_si128();
    X[14] = _mm_set1_epi32(0x00000100);

    __m128i h0 = _mm_set1_epi32(0x67452301), h1 = _mm_set1_epi32(0xEFCDAB89),
            h2 = _mm_set1_epi32(0x98BADCFE), h3 = _mm_set1_epi32(0x10325476),
            h4 = _mm_set1_epi32(0xC3D2E1F0);
    __m128i al = h0, bl = h1, cl = h2, dl = h3, el = h4;
    __m128i ar = h0, br = h1, cr = h2, dr = h3, er = h4;

    for (int j = 0; j < 80; j++) {
        int round = j >> 4;
        __m128i tl = _mm_add_epi32(_mm_add_epi32(_mm_add_epi32(al, fr(round, bl, cl, dl)), X[RL[j]]),
                                   _mm_set1_epi32((int)KL[round]));
        tl = _mm_add_epi32(rotl_epi32(tl, SL[j]), el);
        al = el; el = dl; dl = rotl_epi32(cl, 10); cl = bl; bl = tl;

        __m128i tr = _mm_add_epi32(_mm_add_epi32(_mm_add_epi32(ar, fr(4 - round, br, cr, dr)), X[RR[j]]),
                                   _mm_set1_epi32((int)KR[round]));
        tr = _mm_add_epi32(rotl_epi32(tr, SR[j]), er);
        ar = er; er = dr; dr = rotl_epi32(cr, 10); cr = br; br = tr;
    }

    __m128i t = _mm_add_epi32(_mm_add_epi32(h1, cl), dr);
    h1 = _mm_add_epi32(_mm_add_epi32(h2, dl), er);
    h2 = _mm_add_epi32(_mm_add_epi32(h3, el), ar);
    h3 = _mm_add_epi32(_mm_add_epi32(h4, al), br);
    h4 = _mm_add_epi32(_mm_add_epi32(h0, bl), cr);
    h0 = t;

    uint32_t o0[4], o1[4], o2[4], o3[4], o4[4];
    _mm_storeu_si128((__m128i *)o0, h0);
    _mm_storeu_si128((__m128i *)o1, h1);
    _mm_storeu_si128((__m128i *)o2, h2);
    _mm_storeu_si128((__m128i *)o3, h3);
    _mm_storeu_si128((__m128i *)o4, h4);
    for (int l = 0; l < 4; l++) {
        uint8_t *o = out + l * 20;
        memcpy(o + 0, &o0[l], 4);  memcpy(o + 4, &o1[l], 4);
        memcpy(o + 8, &o2[l], 4);  memcpy(o + 12, &o3[l], 4);
        memcpy(o + 16, &o4[l], 4);
    }
}

static void hash160_x4(const uint8_t *pub, uint8_t *out20) {
    uint8_t sha[4 * 32];
    for (int l = 0; l < 4; l++) sha256_33(pub + l * 33, sha + l * 32);
    ripemd160_x4(sha, out20);
}

static void hash160_x4_uncomp(const uint8_t *pub, uint8_t *out20) {
    uint8_t sha[4 * 32];
    for (int l = 0; l < 4; l++) sha256_65(pub + l * 65, sha + l * 32);
    ripemd160_x4(sha, out20);
}

void hash160_many(const uint8_t *pub, uint8_t *out20, size_t n) {
    size_t full = n & ~(size_t)3;
    for (size_t i = 0; i < full; i += 4) {
        hash160_x4(pub + i * 33, out20 + i * 20);
    }
    size_t rem = n - full;
    if (rem) {
        uint8_t tmp_in[4 * 33];
        uint8_t tmp_out[4 * 20];
        for (int l = 0; l < 4; l++) {
            size_t src = (l < (int)rem) ? full + l : n - 1;
            memcpy(tmp_in + l * 33, pub + src * 33, 33);
        }
        hash160_x4(tmp_in, tmp_out);
        for (size_t l = 0; l < rem; l++) {
            memcpy(out20 + (full + l) * 20, tmp_out + l * 20, 20);
        }
    }
}

void hash160_many_uncomp(const uint8_t *pub, uint8_t *out20, size_t n) {
    size_t full = n & ~(size_t)3;
    for (size_t i = 0; i < full; i += 4) {
        hash160_x4_uncomp(pub + i * 65, out20 + i * 20);
    }
    size_t rem = n - full;
    if (rem) {
        uint8_t tmp_in[4 * 65];
        uint8_t tmp_out[4 * 20];
        for (int l = 0; l < 4; l++) {
            size_t src = (l < (int)rem) ? full + l : n - 1;
            memcpy(tmp_in + l * 65, pub + src * 65, 65);
        }
        hash160_x4_uncomp(tmp_in, tmp_out);
        for (size_t l = 0; l < rem; l++) {
            memcpy(out20 + (full + l) * 20, tmp_out + l * 20, 20);
        }
    }
}

#endif /* __x86_64__ */
