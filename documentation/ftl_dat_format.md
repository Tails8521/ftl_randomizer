* All numbers are big endian
* Sections are 4 bytes aligned (only relevant for the filepaths section which shall be padded with null bytes if needed)
* Header section: 16 bytes

  * 4 bytes magic: "PKG" followed by 0x0a
  * 2 bytes header size (always 16)
  * 2 bytes index entry size (always 20)
  * 4 bytes index entries count
  * 4 bytes filepaths section byte count
* Index entries section: 20 bytes per entry

  * 4 bytes checksum
  * 4 bytes path offset (Offset from the start of the filepaths section to the start of the file path string)
  * 4 bytes offset (Absolute offset to the file data)
  * 4 bytes length
  * 4 bytes filesize
* filepaths section

  * Contains a pool of all filepaths.
  * Each is an ASCII C string (null terminated).
  * Paths are not necessarily stored in the order of the index entries.
* file data section

  * raw bytes