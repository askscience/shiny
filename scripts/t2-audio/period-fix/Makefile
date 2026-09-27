# DKMS build for the patched t2bce_audio module.
obj-m += t2bce_audio/

# The driver includes t2bce_core_transport.h from its sibling directory, exactly
# as the in-tree build does.
ccflags-y += -I$(src)/t2bce_core/include

# Silence an unused-parameter warning the vendored source triggers; mirrors the
# in-tree build which sets this too.
ccflags-y += -Wno-unused-parameter
