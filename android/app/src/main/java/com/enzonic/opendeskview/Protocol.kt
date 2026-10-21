package com.enzonic.opendeskview

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

/**
 * Clean-room spacedesk wire protocol (v4.8). Mirrors crates/spacedesk-proto; see docs/PROTOCOL.md.
 * Every message is a fixed 128-byte little-endian header, optionally followed by a payload.
 */
object Protocol {
    const val HEADER_LEN = 128
    const val DEFAULT_PORT = 28252
    const val PROTO_MAJOR = 4
    const val PROTO_MINOR = 8
    const val IDENT_PAYLOAD_LEN = 334

    val DISCOVERY_MAGIC: ByteArray =
        "SPACEDESK-NET-CLIENT\u0000".toByteArray(StandardCharsets.US_ASCII)

    // Message types (header offset 0).
    const val T_IDENTIFICATION = 0
    const val T_PING = 1
    const val T_FRAMEBUFFER = 2
    const val T_VISIBILITY = 3
    const val T_FLOW_ACK = 7
    const val T_DISCONNECT = 8
    const val T_MOUSE = 10
    const val T_KEYBOARD = 11
    const val T_TOUCH = 12

    // Encodings.
    const val ENC_RAW_BGRX = 0
    const val ENC_JPEG = 3
    const val ENC_H264 = 4

    // Touch flags.
    const val TOUCH_ABSOLUTE = 0x01
    const val TOUCH_DOWN = 0x02
    const val TOUCH_UP = 0x04

    private fun header(type: Int): ByteBuffer =
        ByteBuffer.allocate(HEADER_LEN).order(ByteOrder.LITTLE_ENDIAN).apply { putInt(0, type) }

    private fun utf16le(s: String): ByteArray = s.toByteArray(StandardCharsets.UTF_16LE)

    /** Identification (type 0): 128-byte header + 334-byte payload = 462 bytes. */
    fun buildIdentification(
        width: Int,
        height: Int,
        quality: Int,
        fps: Int,
        encoding: Int,
        guid: String,
        deviceName: String,
    ): ByteArray {
        val h = header(T_IDENTIFICATION)
        h.putInt(4, IDENT_PAYLOAD_LEN)
        h.putInt(8, PROTO_MAJOR)
        h.putInt(12, PROTO_MINOR)
        h.putInt(16, 0)      // client type: display (WindowsRemoteMonitor)
        h.putInt(20, 3)
        h.putInt(24, 3)
        h.putInt(28, 2)
        h.putInt(32, quality)
        h.putShort(36, encoding.toShort())
        h.put(38, 0x01)      // extended-identification flag
        h.putShort(44, fps.toShort())
        h.putShort(46, 4)
        h.putInt(48, 1)      // one resolution follows
        h.putInt(52, width)
        h.putInt(88, height)

        val pay = ByteArrayOutputStream(IDENT_PAYLOAD_LEN)
        val one = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(1).array()
        pay.write(one)
        pay.write(utf16le(guid))
        pay.write(byteArrayOf(0, 0))
        pay.write(utf16le(deviceName))
        val payload = pay.toByteArray().copyOf(IDENT_PAYLOAD_LEN) // zero-pad / clamp to 334

        return h.array() + payload
    }

    private fun headerOnly(type: Int): ByteArray = header(type).array()

    fun buildPing(): ByteArray = headerOnly(T_PING)
    fun buildFlowAck(): ByteArray = headerOnly(T_FLOW_ACK)
    fun buildDisconnect(): ByteArray = headerOnly(T_DISCONNECT)

    /** Touch (type 12), header-only. Coordinates are in [0, resX] x [0, resY]. */
    fun buildTouch(x: Int, y: Int, resX: Int, resY: Int, flags: Int, ts: Int): ByteArray {
        val h = header(T_TOUCH)
        h.putInt(8, x)
        h.putInt(12, y)
        h.putInt(16, resX)
        h.putInt(20, resY)
        h.putInt(24, flags)
        h.putInt(28, ts)
        return h.array()
    }

    /** Parsed framebuffer header (type 2). */
    class Framebuffer(h: ByteBuffer) {
        val width = h.getInt(8)
        val height = h.getInt(12)
        val pitch = h.getInt(16)
        val pixelFormat = h.getInt(20)
        val rectLeft = h.getInt(24)
        val rectTop = h.getInt(28)
        val rectRight = h.getInt(32)
        val rectBottom = h.getInt(36)
        val encoding = h.getInt(40)
        val fragmentCount = h.getInt(60)
        val fragmentInfo = h.getInt(64)
    }
}
