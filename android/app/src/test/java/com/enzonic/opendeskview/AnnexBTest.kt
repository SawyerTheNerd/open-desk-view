package com.enzonic.opendeskview

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AnnexBTest {
    private fun au(vararg nal: Pair<Boolean, Int>): ByteArray =
        nal.flatMap { (long, header) ->
            (if (long) listOf(0, 0, 0, 1) else listOf(0, 0, 1)) + listOf(header, 0x88, 0x84)
        }.map { it.toByte() }.toByteArray()

    @Test
    fun idrWithParameterSetsIsAKeyframe() {
        // SPS (0x67), PPS (0x68), IDR slice (0x65), as OpenH264 emits them.
        assertTrue(AnnexB.isKeyframe(au(true to 0x67, true to 0x68, true to 0x65)))
    }

    @Test
    fun bareIdrSliceIsAKeyframe() {
        assertTrue(AnnexB.isKeyframe(au(false to 0x65)))
    }

    @Test
    fun pFrameIsNot() {
        assertFalse(AnnexB.isKeyframe(au(true to 0x41)))
        // An IDR after a non-IDR slice would be malformed; stop at the first slice.
        assertFalse(AnnexB.isKeyframe(au(true to 0x41, true to 0x65)))
    }

    @Test
    fun emptyOrGarbageIsNot() {
        assertFalse(AnnexB.isKeyframe(ByteArray(0)))
        assertFalse(AnnexB.isKeyframe(byteArrayOf(1, 2, 3, 4, 5)))
    }
}
