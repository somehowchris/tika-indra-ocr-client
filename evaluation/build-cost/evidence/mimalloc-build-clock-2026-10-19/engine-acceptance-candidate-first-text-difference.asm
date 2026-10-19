
/vol/md1/tika-indra-artifacts/frozen-wp1-ad24c12-20261019/candidate/engine-acceptance/engine-acceptance:     file format elf64-x86-64


Disassembly of section .text:

0000000000177c80 <.text+0x2180>:
  177c80:	08 48 8b             	or     %cl,-0x75(%rax)
  177c83:	05 90 4a 50 00       	add    $0x504a90,%eax
  177c88:	48 89 44 24 10       	mov    %rax,0x10(%rsp)
  177c8d:	48 8d 3d 1f 9b ed ff 	lea    -0x1264e1(%rip),%rdi        # 0x517b3
  177c94:	48 8d 74 24 08       	lea    0x8(%rsp),%rsi
  177c99:	e8 d2 ee 1b 00       	call   0x336b70
  177c9e:	84 c0                	test   %al,%al
  177ca0:	75 4d                	jne    0x177cef
  177ca2:	48 8b 05 77 4a 50 00 	mov    0x504a77(%rip),%rax        # 0x67c720
  177ca9:	48 89 44 24 18       	mov    %rax,0x18(%rsp)
  177cae:	48 8d 44 24 18       	lea    0x18(%rsp),%rax
  177cb3:	48 89 44 24 20       	mov    %rax,0x20(%rsp)
  177cb8:	48 8d 35 f4 9a ed ff 	lea    -0x12650c(%rip),%rsi        # 0x517b3
  177cbf:	48 8d 7c 24 20       	lea    0x20(%rsp),%rdi
  177cc4:	48 8d 54 24 08       	lea    0x8(%rsp),%rdx
  177cc9:	ff 15 59 4a 50 00    	call   *0x504a59(%rip)        # 0x67c728
  177ccf:	48                   	rex.W
