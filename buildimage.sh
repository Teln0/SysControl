# Build the .elf file
set -e

trap 'echo "\"${last_command}\" command filed with exit code $?."' EXIT

make clean
make all

rm -rf ./SysControl.hdd

# Create an empty zeroed out 64MiB image file.
dd if=/dev/zero bs=1M count=0 seek=64 of=SysControl.hdd

# Create a GPT partition layout.
parted -s SysControl.hdd mklabel gpt

# Create a partition that spans the whole disk.
parted -s SysControl.hdd mkpart primary 2048s 100%

# Format this new GPT partition as echfs (blocks of 512 bytes in size).
# -g stands for GPT, use -m alternatively for MBR.
echfs-utils -g -p0 SysControl.hdd quick-format 512

# Copy config file and kernel file(s) over into the image.
echfs-utils -g -p0 SysControl.hdd import limine.cfg limine.cfg
echfs-utils -g -p0 SysControl.hdd import SysControl.elf SysControl.elf

# Finally, install Limine onto the image.
limine-install SysControl.hdd

qemu-system-x86_64 ./SysControl.hdd -no-shutdown -no-reboot -m 500M