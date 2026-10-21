package com.enzonic.opendeskview

import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.nio.charset.StandardCharsets

/** LAN discovery: broadcast the probe, collect replying servers. */
object Discovery {

    data class Server(val ip: String, val name: String)

    /** Broadcast and listen for [waitMs]. Must be called off the main thread. */
    fun discover(waitMs: Int = 1200): List<Server> {
        val found = LinkedHashMap<String, Server>()
        val sock = DatagramSocket()
        try {
            sock.broadcast = true
            sock.soTimeout = 250
            val bcast = InetAddress.getByName("255.255.255.255")
            repeat(5) {
                val p = DatagramPacket(
                    Protocol.DISCOVERY_MAGIC, Protocol.DISCOVERY_MAGIC.size, bcast, Protocol.DEFAULT_PORT
                )
                try {
                    sock.send(p)
                } catch (_: Exception) {
                }
            }
            val buf = ByteArray(2048)
            val deadline = System.currentTimeMillis() + waitMs
            while (System.currentTimeMillis() < deadline) {
                val reply = DatagramPacket(buf, buf.size)
                try {
                    sock.receive(reply)
                    val ip = reply.address.hostAddress ?: continue
                    val name = extractName(buf, reply.length)
                    found.putIfAbsent(ip, Server(ip, name))
                } catch (_: Exception) {
                    // timeout tick
                }
            }
        } finally {
            sock.close()
        }
        return found.values.toList()
    }

    /** Longest run of printable UTF-16LE characters in the reply = the computer name. */
    private fun extractName(buf: ByteArray, len: Int): String {
        var best = StringBuilder()
        var cur = StringBuilder()
        var i = 0
        while (i + 1 < len) {
            val lo = buf[i].toInt() and 0xFF
            val hi = buf[i + 1].toInt() and 0xFF
            if (hi == 0 && (lo in 0x20..0x7E)) {
                cur.append(lo.toChar())
            } else {
                if (cur.length > best.length) best = StringBuilder(cur)
                cur = StringBuilder()
            }
            i += 2
        }
        if (cur.length > best.length) best = cur
        return best.toString().trim()
    }
}
