
/vol/md1/tika-indra-artifacts/frozen-wp1-ad24c12-20261019/baseline/engine-acceptance/ocr:     file format elf64-x86-64


Disassembly of section .text:

00000000007706a7 <.text+0x4d27>:
  7706a7:	00 48 89             	add    %cl,-0x77(%rax)
  7706aa:	44 24 28             	rex.R and $0x28,%al
  7706ad:	48 8d 35 64 59 96 02 	lea    0x2965964(%rip),%rsi        # 0x30d6018
  7706b4:	48 8d 15 7c 09 9d ff 	lea    -0x62f684(%rip),%rdx        # 0x141037
  7706bb:	4c 8d 74 24 70       	lea    0x70(%rsp),%r14
  7706c0:	48 8d 4c 24 10       	lea    0x10(%rsp),%rcx
  7706c5:	4c 89 f7             	mov    %r14,%rdi
  7706c8:	ff 15 b2 79 9f 02    	call   *0x29f79b2(%rip)        # 0x3168080
  7706ce:	48 8d bc 24 a8 00 00 	lea    0xa8(%rsp),%rdi
  7706d5:	00 
  7706d6:	ba 08 00 00 00       	mov    $0x8,%edx
  7706db:	4c 89 f6             	mov    %r14,%rsi
  7706de:	31 c9                	xor    %ecx,%ecx
  7706e0:	e8 3b 43 00 00       	call   0x774a20
  7706e5:	4c 89 f7             	mov    %r14,%rdi
  7706e8:	e8 13 6e 00 00       	call   0x777500
  7706ed:	48 8b 84 24 80 00 00 	mov    0x80(%rsp),%rax
  7706f4:	00 
  7706f5:	48                   	rex.W
  7706f6:	89                   	.byte 0x89
